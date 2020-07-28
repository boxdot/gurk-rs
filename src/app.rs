use crate::config::{self, Config};
use crate::signal;
use crate::util::StatefulList;

use anyhow::Context;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use crossterm::event::KeyCode;
use serde::{Deserialize, Serialize};

use std::fs::File;
use std::io::Write;
use std::path::Path;

pub struct App {
    pub config: Config,
    pub should_quit: bool,
    pub log_file: File,
    pub signal_client: signal::SignalClient,
    pub data: AppData,
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
        data.input_cursor = data.input.len();
        Ok(data)
    }

    fn init_from_signal(client: &signal::SignalClient) -> anyhow::Result<Self> {
        let groups = client
            .get_groups()
            .context("failed to get groups from signal")?;
        let channels = groups.into_iter().map(|group_info| {
            let name = group_info
                .name
                .as_ref()
                .unwrap_or_else(|| &group_info.group_id)
                .to_string();
            Channel {
                id: group_info.group_id,
                name,
                is_group: true,
                messages: Vec::new(),
                unread_messages: 0,
            }
        });
        let mut channels = StatefulList::with_items(channels.collect());
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
    #[serde(default)]
    pub attachments: Vec<signal::Attachment>,
    pub arrived_at: DateTime<Utc>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event<I> {
    Input(I),
    Message {
        /// used for debugging
        payload: String,
        /// some message if deserialized successfully
        message: Option<signal::Message>,
    },
    Resize,
}

impl App {
    pub fn try_new() -> anyhow::Result<Self> {
        let config_path = config::installed_config().expect("missing default location for config");
        let config = config::load_from(&config_path)
            .with_context(|| format!("failed to read config from: {}", config_path.display()))?;

        let log_file = File::create("gurk.log").unwrap();

        let mut data = match AppData::load(&config.data_path) {
            Ok(data) => data,
            Err(_) => {
                let client = signal::SignalClient::from_config(config.clone());
                let data = AppData::init_from_signal(&client)?;
                data.save(&config.data_path)?;
                data
            }
        };
        if data.channels.state.selected().is_none() {
            data.channels.state.select(Some(0));
            data.save(&config.data_path)?;
        }

        let signal_client = signal::SignalClient::from_config(config.clone());

        Ok(Self {
            config,
            data,
            should_quit: false,
            signal_client,
            log_file,
        })
    }

    pub fn on_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char(c) => {
                self.data.input.insert(self.data.input_cursor, c);
                self.data.input_cursor += 1;
            }
            KeyCode::Enter if !self.data.input.is_empty() => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.send_input(idx)
                }
            }
            KeyCode::Backspace => {
                if self.data.input_cursor > 0 && self.data.input_cursor < self.data.input.len() + 1
                {
                    self.data.input.remove(self.data.input_cursor - 1);
                    self.data.input_cursor = self.data.input_cursor.saturating_sub(1);
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
            signal::SignalClient::send_message(&message, &channel.id);
        } else {
            signal::SignalClient::send_group_message(&message, &channel.id);
        }

        channel.messages.push(Message {
            from: self.config.user.name.clone(),
            message: Some(message),
            attachments: Vec::new(),
            arrived_at: Utc::now(),
        });

        self.bubble_up_channel(channel_idx);
        self.save().unwrap();
    }

    pub fn on_up(&mut self) {
        self.data.channels.previous();
        // self.save().unwrap();
    }

    pub fn on_down(&mut self) {
        self.data.channels.next();
        // self.save().unwrap();
    }

    pub fn on_left(&mut self) {
        self.data.input_cursor = self.data.input_cursor.saturating_sub(1);
    }

    pub fn on_right(&mut self) {
        if self.data.input_cursor < self.data.input.len() {
            self.data.input_cursor += 1;
        }
    }

    #[allow(dead_code)]
    pub fn log(&mut self, msg: impl AsRef<str>) {
        writeln!(&mut self.log_file, "{}", msg.as_ref()).unwrap();
    }

    pub async fn on_message(
        &mut self,
        message: Option<signal::Message>,
        payload: String,
    ) -> Option<()> {
        self.log(format!("incoming: {} -> {:?}", payload, message));
        let mut message = message?;

        let mut msg: signal::InnerMessage = message
            .envelope
            .sync_message
            .take()
            .map(|m| m.sent_message)
            .or_else(|| message.envelope.data_message.take())?;

        // message text + attachments paths
        let text = msg.message.take();
        let attachments = msg.attachments.take().unwrap_or_default();
        if text.is_none() && attachments.is_empty() {
            return None;
        }

        let channel_id = msg
            .group_info
            .as_ref()
            .map(|g| g.group_id.as_str())
            .or_else(|| {
                if message.envelope.source == self.config.user.phone_number {
                    msg.destination.as_deref()
                } else {
                    Some(message.envelope.source.as_str())
                }
            })?
            .to_string();
        let is_group = msg.group_info.is_some();

        let arrived_at = NaiveDateTime::from_timestamp(
            message.envelope.timestamp as i64 / 1000,
            (message.envelope.timestamp % 1000) as u32,
        );
        let arrived_at = Utc.from_utc_datetime(&arrived_at);

        let name = self
            .resolve_contact_name(message.envelope.source.clone())
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
        }

        self.bubble_up_channel(channel_idx);
        self.save().unwrap();

        Some(())
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
