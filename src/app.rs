use crate::config::{self, Config};
use crate::signal;
use crate::util::{self, StatefulList};

use anyhow::Context;
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use libsignal_service::{
    content::{ContentBody, Metadata},
    proto::GroupContextV2,
    ServiceAddress,
};
use libsignal_service::{prelude::Content, proto::DataMessage};
use log::error;
use notify_rust::Notification;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use std::collections::HashMap;
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
    pub names: HashMap<Uuid, String>,
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

#[derive(Serialize, Deserialize)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    #[serde(default)]
    pub group_data: Option<GroupData>,
    #[serde(serialize_with = "Channel::serialize_msgs")]
    #[serde(deserialize_with = "Channel::deserialize_msgs")]
    pub messages: StatefulList<Message>,
    #[serde(default)]
    pub unread_messages: usize,
}

#[derive(Serialize, Deserialize)]
pub struct GroupData {
    pub members: Vec<Uuid>,
    pub revision: u32,
}

impl Channel {
    /// Used in UI when there is no channel selected.
    pub fn empty() -> Self {
        Channel {
            id: Uuid::default().into(),
            name: " ".to_string(),
            group_data: None,
            messages: util::StatefulList::with_items(Vec::new()),
            unread_messages: 0,
        }
    }

    fn user_id(&self) -> Option<Uuid> {
        match self.id {
            ChannelId::User(id) => Some(id),
            ChannelId::Group(_) => None,
        }
    }

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

#[derive(Serialize, Deserialize, PartialEq, Eq)]
pub enum ChannelId {
    User(Uuid),
    Group(Vec<u8>),
}

impl From<Uuid> for ChannelId {
    fn from(id: Uuid) -> Self {
        ChannelId::User(id)
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

        let timestamp = util::utc_timestamp_msec();
        let mut data_message = DataMessage {
            body: Some(message.clone()),
            timestamp: Some(timestamp),
            ..Default::default()
        };

        match channel.id {
            ChannelId::User(uuid) => {
                let manager = self.signal_manager.clone();
                let body = ContentBody::DataMessage(data_message);
                tokio::task::spawn_local(async move {
                    if let Err(e) = manager.send_message(uuid, body, timestamp).await {
                        // TODO: Proper error handling
                        log::error!("Failed to send message to {}: {}", uuid, e);
                        return;
                    }
                });
            }
            ChannelId::Group(ref master_key) => {
                if let Some(group_data) = channel.group_data.as_ref() {
                    let manager = self.signal_manager.clone();
                    let self_uuid = self.signal_manager.uuid();

                    data_message.group_v2 = Some(GroupContextV2 {
                        master_key: Some(master_key.clone()),
                        revision: Some(group_data.revision),
                        ..Default::default()
                    });

                    let recipients = group_data.members.clone().into_iter();

                    tokio::task::spawn_local(async move {
                        let recipients =
                            recipients.filter(|uuid| *uuid != self_uuid).map(Into::into);
                        if let Err(e) = manager
                            .send_message_to_group(recipients, data_message, timestamp)
                            .await
                        {
                            // TODO: Proper error handling
                            log::error!("Failed to send group message: {}", e);
                            return;
                        }
                    });
                } else {
                    error!("cannot send to broken channel without group data");
                }
            }
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

    pub async fn on_message(&mut self, content: Content) -> anyhow::Result<()> {
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
                    arrived_at: util::timestamp_msec_to_utc(timestamp),
                };
                self.add_message_to_channel(channel_idx, message);
            }
            // Direct/group message by us from a different device
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
                            destination_e164,
                            destination_uuid,
                            timestamp: Some(timestamp),
                            message:
                                Some(DataMessage {
                                    body: Some(text),
                                    group_v2,
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if sender_uuid == self_uuid => {
                let from_id = self_uuid;
                let from = self.config.user.name.clone();

                let channel_idx = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    // message -> group
                    self.ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?
                } else if let (Some(destination_uuid), Some(destination_e164)) = (
                    destination_uuid.and_then(|s| s.parse().ok()),
                    destination_e164,
                ) {
                    // message -> contact
                    self.ensure_contact_channel_exists(destination_uuid, &destination_e164)
                        .await
                } else {
                    return Ok(());
                };

                let message = Message {
                    from_id,
                    from,
                    message: Some(text),
                    attachments: Default::default(),
                    arrived_at: util::timestamp_msec_to_utc(timestamp),
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
                let name = self
                    .ensure_user_is_known(uuid, profile_key, phone_number)
                    .await
                    .to_string();
                let channel_idx = self.ensure_contact_channel_exists(uuid, &name).await;
                let from = self.data.channels.items[channel_idx].name.clone();
                self.notify(&from, &text);
                let message = Message {
                    from_id: uuid,
                    from,
                    message: Some(text),
                    attachments: Default::default(),
                    arrived_at: util::timestamp_msec_to_utc(timestamp),
                };
                self.add_message_to_channel(channel_idx, message);
            }
            // Group message
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
                    group_v2:
                        Some(GroupContextV2 {
                            master_key: Some(master_key),
                            revision: Some(revision),
                            ..
                        }),
                    timestamp: Some(timestamp),
                    profile_key: Some(profile_key),
                    ..
                }),
            ) => {
                let channel_idx = self
                    .ensure_group_channel_exists(master_key, revision)
                    .await
                    .context("failed to create group channel")?;
                let from = self
                    .ensure_user_is_known(uuid, profile_key, phone_number)
                    .await
                    .to_string();
                self.notify(&from, &text);
                let message = Message {
                    from_id: uuid,
                    from,
                    message: Some(text),
                    attachments: Default::default(),
                    arrived_at: util::timestamp_msec_to_utc(timestamp),
                };
                self.add_message_to_channel(channel_idx, message);
            }
            _ => (),
        };

        Ok(())
    }

    async fn ensure_group_channel_exists(
        &mut self,
        master_key: Vec<u8>,
        revision: u32,
    ) -> anyhow::Result<usize> {
        let id = ChannelId::Group(master_key.clone());
        if let Some(channel_idx) = self
            .data
            .channels
            .items
            .iter()
            .position(|channel| channel.id == id)
        {
            let is_stale = match self.data.channels.items[channel_idx].group_data.as_ref() {
                Some(group_data) => group_data.revision != revision,
                None => true,
            };
            if is_stale {
                let (name, group_data, profile_keys) =
                    signal::try_resolve_group(&mut self.signal_manager, master_key).await?;

                self.try_ensure_users_are_known(
                    group_data
                        .members
                        .iter()
                        .copied()
                        .zip(profile_keys.into_iter()),
                )
                .await;

                let channel = &mut self.data.channels.items[channel_idx];
                channel.name = name;
                channel.group_data = Some(group_data);
            }
            Ok(channel_idx)
        } else {
            let (name, group_data, profile_keys) =
                signal::try_resolve_group(&mut self.signal_manager, master_key).await?;

            self.try_ensure_users_are_known(
                group_data
                    .members
                    .iter()
                    .copied()
                    .zip(profile_keys.into_iter()),
            )
            .await;

            self.data.channels.items.push(Channel {
                id,
                name,
                group_data: Some(group_data),
                messages: StatefulList::with_items(Vec::new()),
                unread_messages: 0,
            });
            Ok(self.data.channels.items.len() - 1)
        }
    }

    async fn ensure_user_is_known(
        &mut self,
        uuid: Uuid,
        profile_key: Vec<u8>,
        fallback_name: impl std::fmt::Display,
    ) -> &str {
        if self
            .try_ensure_user_is_known(uuid, profile_key)
            .await
            .is_none()
        {
            self.data.names.insert(uuid, fallback_name.to_string());
        }
        self.data.names.get(&uuid).unwrap()
    }

    async fn try_ensure_user_is_known(&mut self, uuid: Uuid, profile_key: Vec<u8>) -> Option<&str> {
        let is_phone_number_or_unknown = self
            .data
            .names
            .get(&uuid)
            .map(|name| name.starts_with('+'))
            .unwrap_or(true);
        if is_phone_number_or_unknown {
            let name = match profile_key.try_into() {
                Ok(key) => signal::contact_name(&self.signal_manager, uuid, key).await,
                Err(_) => None,
            };
            self.data.names.insert(uuid, name?);
        }
        self.data.names.get(&uuid).map(|s| s.as_str())
    }

    async fn try_ensure_users_are_known(
        &mut self,
        users_with_keys: impl Iterator<Item = (Uuid, Vec<u8>)>,
    ) {
        // TODO: Run in parallel
        for (uuid, profile_key) in users_with_keys {
            self.try_ensure_user_is_known(uuid, profile_key).await;
        }
    }

    fn ensure_own_channel_exists(&mut self) -> usize {
        let self_uuid = self.signal_manager.uuid();
        if let Some(channel_idx) = self
            .data
            .channels
            .items
            .iter_mut()
            .position(|channel| channel.user_id() == Some(self_uuid))
        {
            channel_idx
        } else {
            self.data.channels.items.push(Channel {
                id: self_uuid.into(),
                name: self.config.user.name.clone(),
                group_data: None,
                messages: StatefulList::with_items(Vec::new()),
                unread_messages: 0,
            });
            self.data.channels.items.len() - 1
        }
    }

    async fn ensure_contact_channel_exists(&mut self, uuid: Uuid, name: &str) -> usize {
        if let Some(channel_idx) = self
            .data
            .channels
            .items
            .iter_mut()
            .position(|channel| channel.user_id() == Some(uuid))
        {
            channel_idx
        } else {
            self.data.channels.items.push(Channel {
                id: uuid.into(),
                name: name.to_string(),
                group_data: None,
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
