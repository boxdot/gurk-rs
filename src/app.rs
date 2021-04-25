use crate::config::{self, Config};
use crate::signal;
use crate::util::StatefulList;

use anyhow::Context;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use crossterm::event::KeyCode;
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;

#[cfg(feature = "notifications")]
use notify_rust::Notification;

use std::collections::HashSet;
use std::fs::File;
use std::path::Path;

type SignalManager = presage::Manager<presage::config::SledConfigStore>;

pub struct App {
    pub config: Config,
    pub should_quit: bool,
    pub signal_manager: SignalManager,
    pub data: AppData,
}

impl App {
    pub fn save(&self) -> anyhow::Result<()> {
        self.data.save(&self.config.data_path)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct AppData {
    pub channels: StatefulList<Channel>,
    #[serde(skip)]
    pub chanpos: ChannelPosition,
    pub input: String,
    /// Input position in bytes (not number of chars)
    #[serde(skip)]
    pub input_cursor: usize,
    /// Input position in chars
    #[serde(skip)]
    pub input_cursor_chars: usize,
}

impl AppData {
    fn save(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let f = File::create(path)?;
        serde_json::to_writer(f, self)?;
        Ok(())
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        log::info!("loading app data from: {}", path.as_ref().display());
        let f = File::open(path)?;
        let mut data: Self = serde_json::from_reader(f)?;
        data.input_cursor = data.input.len();
        data.input_cursor_chars = data.input.width();
        Ok(data)
    }

    pub fn init_from_signal(client: &signal::SignalClient) -> anyhow::Result<Self> {
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
                messages: StatefulList::with_items(Vec::new()),
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
            messages: StatefulList::with_items(Vec::new()),
            unread_messages: 0,
        });

        let mut channels: Vec<_> = group_channels.chain(contact_channels).collect();
        channels.sort_unstable_by(|a, b| a.name.cmp(&b.name));

        let mut channels = StatefulList::with_items(channels);
        if !channels.items.is_empty() {
            channels.state.select(Some(0));
        }

        let chanpos = ChannelPosition {
            top: 0,
            upside: 0,
            // value will be initialized in main.rs
            downside: 0,
        };

        Ok(AppData {
            channels,
            chanpos,
            input: String::new(),
            input_cursor: 0,
            input_cursor_chars: 0,
        })
    }
}

#[derive(Derivative, Serialize, Deserialize)]
#[derivative(Debug)]
pub struct Channel {
    /// Either phone number or group id
    pub id: String,
    pub name: String,
    pub is_group: bool,
    #[derivative(Debug = "ignore")]
    #[serde(serialize_with = "Channel::serialize_msgs")]
    #[serde(deserialize_with = "Channel::deserialize_msgs")]
    pub messages: StatefulList<Message>,
    #[serde(default)]
    pub unread_messages: usize,
}

impl Channel {
    fn serialize_msgs<S>(messages: &StatefulList<Message>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        // the messages StatefulList becomes the vec that was messages.items
        messages.items.serialize(ser)
    }

    fn deserialize_msgs<'de, D>(deserializer: D) -> Result<StatefulList<Message>, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let tmp: Vec<Message> = serde::de::Deserialize::deserialize(deserializer)?;
        Ok(StatefulList::with_items(tmp))
    }
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
pub enum Event<I, C> {
    Click(C),
    Input(I),
    Channels {
        remote: Vec<Channel>,
    },
    Message {
        /// used for debugging
        payload: String,
        /// some message if deserialized successfully
        message: Option<signal::Message>,
    },
    PresageMessage(libsignal_service::content::Content),
    Resize {
        cols: u16,
        rows: u16,
    },
    Quit(Option<anyhow::Error>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelPosition {
    pub top: usize,    // list index of channel at top of viewport
    pub upside: u16,   // number of rows between selected channel and top of viewport
    pub downside: u16, // number of rows between selected channel and bottom of viewport
}

impl Default for ChannelPosition {
    fn default() -> ChannelPosition {
        ChannelPosition {
            top: 0,
            upside: 0,
            downside: 0,
        }
    }
}

fn get_signal_manager() -> anyhow::Result<SignalManager> {
    let data_dir = config::default_data_dir();
    let db_path = data_dir.join("signal-db");
    let config_store = presage::config::SledConfigStore::new(db_path)?;
    let signal_context =
        libsignal_protocol::Context::new(libsignal_protocol::crypto::DefaultCrypto::default())?;
    let manager = presage::Manager::with_config_store(config_store, signal_context)?;
    Ok(manager)
}

async fn ensure_linked_device() -> anyhow::Result<(SignalManager, config::Config)> {
    let mut manager = get_signal_manager()?;
    let config = if let Some(config_path) = config::installed_config() {
        config::load_from(config_path)?
    } else {
        if manager.phone_number().is_none() {
            // link device
            let at_hostname = hostname::get()
                .ok()
                .and_then(|hostname| {
                    hostname
                        .to_string_lossy()
                        .split('.')
                        .filter(|s| !s.is_empty())
                        .next()
                        .map(|s| format!("@{}", s))
                })
                .unwrap_or_else(String::new);
            let device_name = format!("gurk{}", at_hostname);
            println!("Linking new device with device name: {}", device_name);
            manager
                .link_secondary_device(
                    libsignal_service::configuration::SignalServers::Production,
                    device_name.clone(),
                )
                .await?;
        }

        let phone_number = manager
            .phone_number()
            .expect("no phone number after device was linked");
        let profile = manager.retrieve_profile().await?;
        let name = profile
            .name
            .map(|name| name.given_name)
            .unwrap_or_else(|| whoami::username());

        let user = config::User {
            name,
            phone_number: phone_number.to_string(),
        };
        let config = config::Config::with_user(user);
        config.save_new().context("failed to init config file")?;

        config
    };

    Ok((manager, config))
}

impl App {
    pub async fn try_new() -> anyhow::Result<Self> {
        let (signal_manager, config) = ensure_linked_device().await?;

        let mut load_data_path = config.data_path.clone();
        if !load_data_path.exists() {
            // try also to load from legacy data path
            if let Some(fallback_data_path) = config::fallback_data_path() {
                load_data_path = fallback_data_path;
            }
        }

        let mut data = AppData::load(&load_data_path).unwrap_or_default();

        // select the first channel if none is selected
        if data.channels.state.selected().is_none() && !data.channels.items.is_empty() {
            data.channels.state.select(Some(0));
            data.save(&config.data_path)?;
        }

        Ok(Self {
            config,
            data,
            should_quit: false,
            signal_manager,
        })
    }

    pub fn put_char(&mut self, c: char) {
        let idx = self.data.input_cursor;
        self.data.input.insert(idx, c);
        self.data.input_cursor += c.len_utf8();
        self.data.input_cursor_chars += 1;
    }

    pub fn on_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('\r') => self.put_char('\n'),
            KeyCode::Enter if !self.data.input.is_empty() => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.send_input(idx)
                }
            }
            KeyCode::Home => self.on_home(),
            KeyCode::End => self.on_end(),
            KeyCode::Backspace => {
                self.on_backspace();
            }
            KeyCode::Char(c) => self.put_char(c),
            _ => {}
        }
    }

    fn send_input(&mut self, channel_idx: usize) {
        let channel = &mut self.data.channels.items[channel_idx];

        let message: String = self.data.input.drain(..).collect();
        self.data.input_cursor = 0;
        self.data.input_cursor_chars = 0;

        if !channel.is_group {
            signal::SignalClient::send_message(&message, &channel.id);
        } else {
            signal::SignalClient::send_group_message(&message, &channel.id);
        }

        channel.messages.items.push(Message {
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

        // when list is about to cycle from top to bottom
        if self.data.channels.state.selected() == Some(0) {
            self.data.chanpos.top =
                self.data.channels.items.len() - self.data.chanpos.downside as usize - 1;
            self.data.chanpos.upside = self.data.chanpos.downside;
            self.data.chanpos.downside = 0;
        } else {
            // viewport scrolls up in list
            if self.data.chanpos.upside == 0 {
                if self.data.chanpos.top > 0 {
                    self.data.chanpos.top -= 1;
                }
            // select scrolls up in viewport
            } else {
                self.data.chanpos.upside -= 1;
                self.data.chanpos.downside += 1;
            }
        }

        self.data.channels.previous();
    }

    pub fn on_down(&mut self) {
        if self.reset_unread_messages() {
            self.save().unwrap();
        }

        // viewport scrolls down in list
        if self.data.chanpos.downside == 0 {
            self.data.chanpos.top += 1;
        // select scrolls down in viewport
        } else {
            self.data.chanpos.upside += 1;
            self.data.chanpos.downside -= 1;
        }

        self.data.channels.next();

        // when list has just cycled from bottom to top
        if self.data.channels.state.selected() == Some(0) {
            self.data.chanpos.top = 0;
            self.data.chanpos.downside = self.data.chanpos.upside;
            self.data.chanpos.upside = 0;
        }
    }

    pub fn on_pgup(&mut self) {
        let select = self.data.channels.state.selected().unwrap_or_default();
        self.data.channels.items[select].messages.next();
    }

    pub fn on_pgdn(&mut self) {
        let select = self.data.channels.state.selected().unwrap_or_default();
        self.data.channels.items[select].messages.previous();
    }

    pub fn reset_unread_messages(&mut self) -> bool {
        if let Some(selected_idx) = self.data.channels.state.selected() {
            if self.data.channels.items[selected_idx].unread_messages > 0 {
                self.data.channels.items[selected_idx].unread_messages = 0;
                return true;
            }
        }
        false
    }

    pub fn on_left(&mut self) -> Option<()> {
        let mut idx = self.data.input_cursor.checked_sub(1)?;
        while !self.data.input.is_char_boundary(idx) {
            idx -= 1;
        }
        self.data.input_cursor = idx;
        self.data.input_cursor_chars -= 1;
        Some(())
    }

    fn word_operation(&mut self, op: impl Fn(&mut App) -> Option<()>) -> Option<()> {
        while op(self).is_some() {
            if self.data.input.as_bytes().get(self.data.input_cursor)? != &b' ' {
                break;
            }
        }
        while op(self).is_some() {
            if self.data.input.as_bytes().get(self.data.input_cursor)? == &b' ' {
                return Some(());
            }
        }
        None
    }

    /// Move a word back
    pub fn on_alt_left(&mut self) {
        self.on_left();
        self.word_operation(Self::on_left);
        if self.data.input.as_bytes().get(self.data.input_cursor) == Some(&b' ') {
            self.on_right();
        }
    }

    /// Move a word forward
    pub fn on_alt_right(&mut self) {
        self.word_operation(Self::on_right);
        while self.data.input.as_bytes().get(self.data.input_cursor) == Some(&b' ') {
            self.on_right();
        }
    }

    pub fn on_home(&mut self) {
        self.data.input_cursor = 0;
        self.data.input_cursor_chars = 0;
    }

    pub fn on_end(&mut self) {
        self.data.input_cursor = self.data.input.len();
        self.data.input_cursor_chars = self.data.input.width();
    }

    pub fn on_right(&mut self) -> Option<()> {
        let mut idx = Some(self.data.input_cursor + 1).filter(|x| x <= &self.data.input.len())?;
        while idx < self.data.input.len() && !self.data.input.is_char_boundary(idx) {
            idx -= 1;
        }
        self.data.input_cursor = idx;
        self.data.input_cursor_chars += 1;
        Some(())
    }

    pub fn on_backspace(&mut self) -> Option<()> {
        let mut idx = self.data.input_cursor.checked_sub(1)?;
        while idx < self.data.input.len() && !self.data.input.is_char_boundary(idx) {
            idx -= 1;
        }
        self.data.input.remove(idx);
        self.data.input_cursor = idx;
        self.data.input_cursor_chars -= 1;
        Some(())
    }

    pub fn on_delete_word(&mut self) -> Option<()> {
        while self
            .data
            .input
            .as_bytes()
            .get(self.data.input_cursor.checked_sub(1)?)?
            == &b' '
        {
            self.on_backspace();
        }
        while self
            .data
            .input
            .as_bytes()
            .get(self.data.input_cursor.checked_sub(1)?)?
            != &b' '
        {
            self.on_backspace();
        }
        Some(())
    }

    pub fn on_delete_suffix(&mut self) {
        if self.data.input_cursor < self.data.input.len() {
            self.data.input.truncate(self.data.input_cursor);
        }
    }

    pub fn on_channels(&mut self, remote_channels: Vec<Channel>) {
        let known_channel_ids: HashSet<String> = self
            .data
            .channels
            .items
            .iter()
            .map(|c| c.id.clone())
            .collect();
        for channel in remote_channels {
            if !known_channel_ids.contains(&channel.id) {
                self.data.channels.items.push(channel)
            }
        }
    }

    pub async fn on_message(
        &mut self,
        message: Option<signal::Message>,
        payload: String,
    ) -> Option<()> {
        log::info!("incoming: {} -> {:?}", payload, message);

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

        let is_from_me = message.envelope.source == self.config.user.phone_number;
        let channel_id = msg
            .group_info
            .as_ref()
            .map(|g| g.group_id.as_str())
            .or_else(|| {
                if is_from_me {
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
                messages: StatefulList::with_items(Vec::new()),
                unread_messages: 0,
            });
            self.data.channels.items.len() - 1
        };

        #[cfg(feature = "notifications")]
        if !is_from_me {
            if let Some(text) = text.as_ref() {
                use std::borrow::Cow;
                let summary = self
                    .data
                    .channels
                    .items
                    .get(channel_idx)
                    .as_ref()
                    .filter(|_| is_group)
                    .map(|c| Cow::from(format!("{} in {}", name, c.name)))
                    .unwrap_or_else(|| Cow::from(&name));
                if let Err(e) = Notification::new().summary(&summary).body(&text).show() {
                    log::error!("failed to send notification: {}", e);
                }
            }
        }

        self.data.channels.items[channel_idx]
            .messages
            .items
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
                for message in channel.messages.items.iter_mut() {
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
                messages: StatefulList::with_items(Vec::new()),
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
