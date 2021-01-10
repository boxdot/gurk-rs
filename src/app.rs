use crate::config::{self, Config};
use crate::signal;
use crate::util::StatefulList;

use anyhow::{anyhow, bail, Context as _};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use crossterm::event::{KeyCode, KeyEvent};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use unicode_width::UnicodeWidthStr;

use std::fs::File;
use std::path::Path;

pub struct App {
    pub config: Config,
    pub should_quit: bool,
    pub signal_client: signal::SignalClient,
    pub data: AppData,
    events_tx: Sender<Event>,
}

impl App {
    fn save(&self) -> anyhow::Result<()> {
        self.data.save(&self.config.data_path)
    }
}

#[derive(Serialize, Deserialize)]
pub struct AppData {
    pub channels: StatefulList<Channel>,
    pub input: String,
    #[serde(skip)]
    pub input_cursor: usize,
}

impl AppData {
    fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let f = File::create(path)?;
        serde_json::to_writer(f, self)?;
        Ok(())
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let f = File::open(path)?;
        let mut data: Self = serde_json::from_reader(f)?;
        data.input_cursor = data.input.width();
        Ok(data)
    }

    fn init_from_signal(client: &signal::SignalClient) -> anyhow::Result<Self> {
        let groups = client
            .get_groups()
            .context("failed to fetch groups from signal")?;
        let group_channels = groups.into_iter().map(|group_info| {
            let name = group_info
                .name
                .as_ref()
                .unwrap_or(&group_info.group_id)
                .to_string();
            Channel {
                id: group_info.group_id,
                name,
                is_group: true,
                messages: Vec::new(),
                unread_messages: 0,
            }
        });

        let contacts = client
            .get_contacts()
            .context("failed to fetch contact from signal")?;
        let contact_channels = contacts.into_iter().map(|contact_info| Channel {
            id: contact_info.phone_number,
            name: contact_info.name,
            is_group: false,
            messages: Vec::new(),
            unread_messages: 0,
        });

        let mut channels: Vec<_> = group_channels.chain(contact_channels).collect();
        channels.sort_unstable_by(|a, b| a.name.cmp(&b.name));

        let mut channels = StatefulList::with_items(channels);
        if !channels.items.is_empty() {
            channels.state.select(Some(0));
        }

        Ok(AppData {
            channels,
            input: String::new(),
            input_cursor: 0,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Channel {
    /// Either phone number or group id
    pub id: String,
    pub name: String,
    pub is_group: bool,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub unread_messages: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub from: String,
    #[serde(alias = "text")] // remove
    pub message: Option<String>,
    #[serde(skip)] // TODO: We need to serde to AttachmentPointer
    pub attachments: Vec<signal::AttachmentPointer>,
    pub arrived_at: DateTime<Utc>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    Input(KeyEvent),
    Message(anyhow::Result<signal::Message>),
    Resize,
    Error(anyhow::Error),
}

impl App {
    pub fn try_new(events_tx: Sender<Event>) -> anyhow::Result<Self> {
        let config_path = config::installed_config()
            .context("config file not found at one of the default locations")?;
        let config = config::load_from(&config_path)
            .with_context(|| format!("failed to read config from: {}", config_path.display()))?;

        let mut load_data_path = config.data_path.clone();
        if !load_data_path.exists() {
            // try also to load from legacy data path
            if let Some(fallback_data_path) = config::fallback_data_path() {
                load_data_path = fallback_data_path;
            }
        }

        let signal_client = signal::SignalClient::with_config(config.clone());

        let mut data = match AppData::load(&load_data_path) {
            Ok(data) => data,
            Err(_) => {
                let data = AppData::init_from_signal(&signal_client)?;
                data.save(&config.data_path)?;
                data
            }
        };
        if data.channels.state.selected().is_none() && !data.channels.items.is_empty() {
            data.channels.state.select(Some(0));
            data.save(&config.data_path)?;
        }

        Ok(Self {
            config,
            data,
            should_quit: false,
            signal_client,
            events_tx,
        })
    }

    pub async fn on_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char(c) => {
                let mut idx = self.data.input_cursor;
                while !self.data.input.is_char_boundary(idx) {
                    idx += 1;
                }
                self.data.input.insert(idx, c);
                self.data.input_cursor += 1;
            }
            KeyCode::Enter if !self.data.input.is_empty() => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.send_input(idx);
                }
            }
            KeyCode::Backspace => {
                if self.data.input_cursor > 0
                    && self.data.input_cursor < self.data.input.width() + 1
                {
                    self.data.input_cursor = self.data.input_cursor.saturating_sub(1);
                    let idx = self
                        .data
                        .input
                        .chars()
                        .take(self.data.input_cursor)
                        .map(|c| c.len_utf8())
                        .sum();
                    self.data.input.remove(idx);
                }
            }
            _ => {}
        }
    }

    fn send_input(&mut self, channel_idx: usize) {
        let channel = &mut self.data.channels.items[channel_idx];

        let message: String = self.data.input.drain(..).collect();
        self.data.input_cursor = 0;

        if !channel.is_group {
            self.signal_client.send_message(
                channel.id.clone(),
                message.clone(),
                self.events_tx.clone(),
            );
        } else {
            self.signal_client.send_group_message(
                channel.id.clone(),
                message.clone(),
                self.events_tx.clone(),
            );
        };

        channel.messages.push(Message {
            from: self.config.user.name.clone(),
            message: Some(message),
            attachments: Vec::new(),
            arrived_at: Utc::now(),
        });

        self.reset_unread_messages();
        self.bubble_up_channel(channel_idx);
        self.save().unwrap();
    }

    pub fn on_up(&mut self) {
        if self.reset_unread_messages() {
            self.save().unwrap();
        }
        self.data.channels.previous();
    }

    pub fn on_down(&mut self) {
        if self.reset_unread_messages() {
            self.save().unwrap();
        }
        self.data.channels.next();
    }

    fn reset_unread_messages(&mut self) -> bool {
        if let Some(selected_idx) = self.data.channels.state.selected() {
            if self.data.channels.items[selected_idx].unread_messages > 0 {
                self.data.channels.items[selected_idx].unread_messages = 0;
                return true;
            }
        }
        false
    }

    pub fn on_left(&mut self) {
        self.data.input_cursor = self.data.input_cursor.saturating_sub(1);
    }

    pub fn on_right(&mut self) {
        if self.data.input_cursor < self.data.input.width() {
            self.data.input_cursor += 1;
        }
    }

    pub async fn on_message(&mut self, message: signal::Message) -> anyhow::Result<()> {
        log::info!("incoming: {:?}", message);

        let mut msg = if let signal::ContentBody::DataMessage(msg) = message.body {
            msg
        } else {
            bail!("only data messages supported at the moment");
        };

        // message text + attachments paths
        let text = msg.body.take();
        let attachments = std::mem::take(&mut msg.attachments);
        if text.is_none() && attachments.is_empty() {
            bail!("no text and attachments fields");
        }

        let channel_id =
            get_channel_id(&message.metadata, &msg).ok_or_else(|| anyhow!("no channel id"))?;
        let is_group = msg.group.is_some() || msg.group_v2.is_some();

        let arrived_at = NaiveDateTime::from_timestamp(
            message.metadata.timestamp as i64 / 1000,
            (message.metadata.timestamp % 1000) as u32,
        );
        let arrived_at = Utc.from_utc_datetime(&arrived_at);

        let name = self
            .resolve_contact_name(
                message
                    .metadata
                    .sender
                    .e164
                    .ok_or_else(|| anyhow!("missing sender.e164 metadata"))?,
            )
            .await;

        let channel_idx = if let Some(channel_idx) = self
            .data
            .channels
            .items
            .iter_mut()
            .position(|channel| channel.id == channel_id && channel.is_group == is_group)
        {
            channel_idx
        } else {
            let channel_name = if is_group {
                let group_name = signal::SignalClient::get_group_name(&channel_id).await;
                group_name.unwrap_or_else(|| channel_id.clone())
            } else {
                name.clone()
            };
            self.data.channels.items.push(Channel {
                id: channel_id.clone(),
                name: channel_name,
                is_group,
                messages: Vec::new(),
                unread_messages: 0,
            });
            self.data.channels.items.len() - 1
        };

        self.data.channels.items[channel_idx]
            .messages
            .push(Message {
                from: name,
                message: text,
                attachments,
                arrived_at,
            });
        if self.data.channels.state.selected() != Some(channel_idx) {
            self.data.channels.items[channel_idx].unread_messages += 1;
        } else {
            self.reset_unread_messages();
        }

        self.bubble_up_channel(channel_idx);
        self.save().unwrap();

        Ok(())
    }

    async fn resolve_contact_name(&mut self, phone_number: String) -> String {
        let contact_channel = self
            .data
            .channels
            .items
            .iter()
            .find(|channel| channel.id == phone_number && !channel.is_group);
        let contact_channel_exists = contact_channel.is_some();
        let name = match contact_channel {
            _ if phone_number == self.config.user.phone_number => {
                Some(self.config.user.name.clone())
            }
            Some(channel) if channel.id == channel.name => {
                signal::SignalClient::get_contact_name(&phone_number).await
            }
            None => signal::SignalClient::get_contact_name(&phone_number).await,
            Some(channel) => Some(channel.name.clone()),
        };

        if let Some(name) = name.as_ref() {
            for channel in self.data.channels.items.iter_mut() {
                for message in channel.messages.iter_mut() {
                    if message.from == phone_number {
                        message.from = name.clone();
                    }
                }
                if channel.id == phone_number {
                    channel.name = name.clone();
                }
            }
        }

        let name = name.unwrap_or_else(|| phone_number.clone());

        if !contact_channel_exists {
            self.data.channels.items.push(Channel {
                id: phone_number,
                name: name.clone(),
                is_group: false,
                messages: Vec::new(),
                unread_messages: 0,
            })
        }

        name
    }

    fn bubble_up_channel(&mut self, channel_idx: usize) {
        // bubble up channel to the beginning of the list
        let channels = &mut self.data.channels;
        for (prev, next) in (0..channel_idx).zip(1..channel_idx + 1).rev() {
            channels.items.swap(prev, next);
        }
        match channels.state.selected() {
            Some(selected_idx) if selected_idx == channel_idx => channels.state.select(Some(0)),
            Some(selected_idx) if selected_idx < channel_idx => {
                channels.state.select(Some(selected_idx + 1));
            }
            _ => {}
        };
    }
}

fn get_channel_id(metadata: &signal::Metadata, msg: &signal::DataMessage) -> Option<String> {
    let group_id = msg
        .group
        .as_ref()
        .and_then(|g| g.id.as_ref())
        .map(hex::encode);
    let group_v2_id = group_id.or_else(|| {
        msg.group_v2
            .as_ref()
            .and_then(|g| g.master_key.as_ref())
            .map(hex::encode)
    });
    group_v2_id.or_else(|| metadata.sender.e164.clone())
}
