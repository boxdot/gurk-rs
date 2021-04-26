use crate::config::{self, Config};
use crate::signal;
use crate::util::StatefulList;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use derivative::Derivative;
use libsignal_service::{
    content::{ContentBody, Metadata},
    ServiceAddress,
};
use libsignal_service::{prelude::Content, proto::DataMessage};
use log::error;
use notify_rust::Notification;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use std::convert::TryInto;
use std::fs::File;
use std::path::Path;

pub struct App {
    pub config: Config,
    pub should_quit: bool,
    pub signal_manager: signal::Manager,
    pub data: AppData,
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
}

#[derive(Derivative, Serialize, Deserialize)]
#[derivative(Debug)]
pub struct Channel {
    /// Either phone number or group id
    pub id: String, // TODO: replace by UUID (groups v1 are gone)
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
    pub from_id: Uuid,
    pub from: String,
    #[serde(alias = "text")] // remove
    pub message: Option<String>,
    #[serde(default)]
    pub attachments: Vec<signal::Attachment>,
    pub arrived_at: DateTime<Utc>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    Click(MouseEvent),
    Input(KeyEvent),
    Message(libsignal_service::content::Content),
    Resize { cols: u16, rows: u16 },
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

impl App {
    pub async fn try_new() -> anyhow::Result<Self> {
        let (signal_manager, config) = signal::ensure_linked_device().await?;

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

    pub fn save(&self) -> anyhow::Result<()> {
        self.data.save(&self.config.data_path)
    }

    pub fn put_char(&mut self, c: char) {
        let idx = self.data.input_cursor;
        self.data.input.insert(idx, c);
        self.data.input_cursor += c.len_utf8();
        self.data.input_cursor_chars += 1;
    }

    pub async fn on_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('\r') => self.put_char('\n'),
            KeyCode::Enter if !self.data.input.is_empty() => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.send_input(idx).await;
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

    async fn send_input(&mut self, channel_idx: usize) {
        let channel = &mut self.data.channels.items[channel_idx];

        let message: String = self.data.input.drain(..).collect();
        self.data.input_cursor = 0;
        self.data.input_cursor_chars = 0;

        if !channel.is_group {
            let uuid: Uuid = channel.id.parse().unwrap();
            let timestamp = crate::util::utc_timestamp_msec();
            let body = ContentBody::DataMessage(DataMessage {
                body: Some(message.clone()),
                timestamp: Some(timestamp),
                ..Default::default()
            });

            let manager = self.signal_manager.clone();
            tokio::task::spawn_local(async move {
                if let Err(e) = manager.send_message(uuid, body, timestamp).await {
                    // TODO: Proper error handling
                    log::error!("Failed to send message to {}: {}", uuid, e);
                    return;
                }
            });
        } else {
            unimplemented!("sending to groups is not yet implemented");
        }

        channel.messages.items.push(Message {
            from_id: self.signal_manager.uuid(),
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

    pub async fn on_message(&mut self, content: Content) {
        use libsignal_service::content::SyncMessage;
        use libsignal_service::proto::sync_message::Sent;

        log::info!("incoming: {:?}", content);

        let self_uuid = self.signal_manager.uuid();

        match (content.metadata, content.body) {
            // Private note message
            (
                _,
                ContentBody::SynchronizeMessage(SyncMessage {
                    sent:
                        Some(Sent {
                            destination_uuid: Some(destination_uuid),
                            timestamp: Some(timestamp),
                            message:
                                Some(DataMessage {
                                    body: Some(text), ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if destination_uuid.parse() == Ok(self_uuid) => {
                let channel_idx = self.ensure_own_channel_exists();
                let message = Message {
                    from_id: self_uuid,
                    from: self.config.user.name.clone(),
                    message: Some(text),
                    attachments: Default::default(),
                    arrived_at: crate::util::timestamp_msec_to_utc(timestamp),
                };
                self.add_message_to_channel(channel_idx, message);
            }
            // Direct message by us from a different device
            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: Some(sender_uuid),
                            ..
                        },
                    ..
                },
                ContentBody::SynchronizeMessage(SyncMessage {
                    sent:
                        Some(Sent {
                            destination_e164: Some(destination_e164),
                            destination_uuid: Some(destination_uuid),
                            timestamp: Some(timestamp),
                            message:
                                Some(DataMessage {
                                    body: Some(text),
                                    group_v2: None,
                                    profile_key: Some(profile_key),
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if sender_uuid == self_uuid => {
                let destination_uuid = destination_uuid.parse().unwrap();
                let channel_idx = self
                    .ensure_contact_channel_exists(destination_uuid, profile_key, destination_e164)
                    .await;
                let message = Message {
                    from_id: destination_uuid,
                    from: self.config.user.name.clone(),
                    message: Some(text),
                    attachments: Default::default(),
                    arrived_at: crate::util::timestamp_msec_to_utc(timestamp),
                };
                self.add_message_to_channel(channel_idx, message);
            }
            // Direct message
            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: Some(uuid),
                            phonenumber: Some(phone_number),
                            ..
                        },
                    ..
                },
                ContentBody::DataMessage(DataMessage {
                    body: Some(text),
                    group_v2: None,
                    timestamp: Some(timestamp),
                    profile_key: Some(profile_key),
                    ..
                }),
            ) => {
                let channel_idx = self
                    .ensure_contact_channel_exists(uuid, profile_key, phone_number)
                    .await;
                let from = self.data.channels.items[channel_idx].name.clone();
                self.notify(&from, &text);
                let message = Message {
                    from_id: uuid,
                    from,
                    message: Some(text),
                    attachments: Default::default(),
                    arrived_at: crate::util::timestamp_msec_to_utc(timestamp),
                };
                self.add_message_to_channel(channel_idx, message);
            }
            _ => return,
        };
    }

    fn ensure_own_channel_exists(&mut self) -> usize {
        let self_uuid = self.signal_manager.uuid().to_string();
        if let Some(channel_idx) = self
            .data
            .channels
            .items
            .iter_mut()
            .position(|channel| channel.id == self_uuid)
        {
            channel_idx
        } else {
            self.data.channels.items.push(Channel {
                id: self_uuid,
                name: self.config.user.name.clone(),
                is_group: false,
                messages: StatefulList::with_items(Vec::new()),
                unread_messages: 0,
            });
            self.data.channels.items.len() - 1
        }
    }

    async fn ensure_contact_channel_exists(
        &mut self,
        uuid: Uuid,
        profile_key: Vec<u8>,
        fallback_name: impl std::fmt::Display,
    ) -> usize {
        let uuid_str = uuid.to_string();
        if let Some(channel_idx) = self
            .data
            .channels
            .items
            .iter_mut()
            .position(|channel| channel.id == uuid_str)
        {
            channel_idx
        } else {
            let name = match profile_key.try_into() {
                Ok(key) => signal::contact_name(&self.signal_manager, uuid, key)
                    .await
                    .unwrap_or_else(|| fallback_name.to_string()),
                Err(_) => fallback_name.to_string(),
            };

            self.data.channels.items.push(Channel {
                id: uuid_str,
                name,
                is_group: false,
                messages: StatefulList::with_items(Vec::new()),
                unread_messages: 0,
            });
            self.data.channels.items.len() - 1
        }
    }

    fn add_message_to_channel(&mut self, channel_idx: usize, message: Message) {
        self.data.channels.items[channel_idx]
            .messages
            .items
            .push(message);
        if self.data.channels.state.selected() != Some(channel_idx) {
            self.data.channels.items[channel_idx].unread_messages += 1;
        } else {
            self.reset_unread_messages();
        }

        self.bubble_up_channel(channel_idx);
        self.save().unwrap();
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

    fn notify(&self, summary: &str, text: &str) {
        if let Err(e) = Notification::new().summary(&summary).body(&text).show() {
            error!("failed to send notification: {}", e);
        }
    }
}
