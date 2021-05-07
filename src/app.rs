use crate::config::{self, Config};
use crate::signal;
use crate::util::{self, StatefulList};

use anyhow::Context;
use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use log::error;
use notify_rust::Notification;
use presage::prelude::{
    content::{ContentBody, DataMessage, Metadata, SyncMessage},
    proto::{
        data_message::{Quote, Reaction},
        sync_message::Sent,
        GroupContextV2,
    },
    Content, ServiceAddress,
};
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
        let f = std::io::BufWriter::new(File::create(path)?);
        serde_json::to_writer(f, self)?;
        Ok(())
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        log::info!("loading app data from: {}", path.as_ref().display());
        let f = std::io::BufReader::new(File::open(path)?);
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

    fn selected_message(&self) -> Option<&Message> {
        // Messages are shown in reversed order => selected is reversed
        self.messages
            .state
            .selected()
            .and_then(|idx| self.messages.items.len().checked_sub(idx + 1))
            .and_then(|idx| self.messages.items.get(idx))
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
    pub message: Option<String>,
    pub arrived_at: u64,
    #[serde(default)]
    pub quote: Option<Box<Message>>,
    #[serde(default)]
    pub attachments: Vec<signal::Attachment>,
    #[serde(default)]
    pub reactions: Vec<(Uuid, String)>,
}

impl Message {
    fn new(from_id: Uuid, message: String, arrived_at: u64) -> Self {
        Self {
            from_id,
            message: Some(message),
            arrived_at,
            quote: None,
            attachments: Default::default(),
            reactions: Default::default(),
        }
    }

    fn from_quote(quote: Quote) -> Option<Message> {
        Some(Message {
            from_id: quote.author_uuid?.parse().ok()?,
            message: quote.text,
            arrived_at: quote.id?,
            quote: None,
            attachments: Default::default(),
            reactions: Default::default(),
        })
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    Redraw,
    Click(MouseEvent),
    Input(KeyEvent),
    Message(Content),
    Resize { cols: u16, rows: u16 },
    Quit(Option<anyhow::Error>),
}

impl App {
    pub async fn try_new(relink: bool) -> anyhow::Result<Self> {
        let (signal_manager, config) = signal::ensure_linked_device(relink).await?;

        let mut load_data_path = config.data_path.clone();
        if !load_data_path.exists() {
            // try also to load from legacy data path
            if let Some(fallback_data_path) = config::fallback_data_path() {
                load_data_path = fallback_data_path;
            }
        }

        // if data file exists, be conservative and fail rather than overriding and losing the messages
        let mut data: AppData;
        if load_data_path.exists() {
            data = AppData::load(&load_data_path).with_context(|| {
                format!(
                    "failed to load stored data from '{}':\n\
            This might happen due to incompatible data model when Gurk is upgraded.\n\
            Please consider to backup your messages and then remove the store.",
                    load_data_path.display()
                )
            })?;
        } else {
            data = AppData::load(&load_data_path).unwrap_or_default();
        }

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

    pub fn name_by_id(&self, id: Uuid) -> &str {
        self.get_name_by_id(id).unwrap_or("Unknown Name")
    }

    pub fn get_name_by_id(&self, id: Uuid) -> Option<&str> {
        self.data.names.get(&id).map(|s| s.as_ref())
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
            KeyCode::Esc => self.reset_message_selection(),
            KeyCode::Char(c) => self.put_char(c),
            _ => {}
        }
    }

    fn reset_message_selection(&mut self) {
        if let Some(idx) = self.data.channels.state.selected() {
            let channel = &mut self.data.channels.items[idx];
            channel.messages.state.select(None);
        }
    }

    async fn send_input(&mut self, channel_idx: usize) {
        let channel = &mut self.data.channels.items[channel_idx];

        let message: String = self.data.input.drain(..).collect();
        self.data.input_cursor = 0;
        self.data.input_cursor_chars = 0;

        let timestamp = util::utc_now_timestamp_msec();

        let quote = channel.selected_message().map(|message| Quote {
            // Messages are shown in reverse order => selected is reverse
            id: Some(message.arrived_at),
            author_uuid: Some(message.from_id.to_string()),
            text: message.message.clone(),
            ..Default::default()
        });
        let quote_message = quote.clone().and_then(Message::from_quote).map(Box::new);
        let with_quote = quote.is_some();

        let mut data_message = DataMessage {
            body: Some(message.clone()),
            timestamp: Some(timestamp),
            quote,
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
            message: Some(message),
            arrived_at: timestamp,
            quote: quote_message,
            attachments: Default::default(),
            reactions: Default::default(),
        });

        self.reset_unread_messages();
        if with_quote {
            self.reset_message_selection();
        }
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
        log::info!("incoming: {:?}", content);

        let self_uuid = self.signal_manager.uuid();

        let (channel_idx, message) = match (content.metadata, content.body) {
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
                let message = Message::new(self_uuid, text, timestamp);
                (channel_idx, message)
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
                                    quote,
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if sender_uuid == self_uuid => {
                let channel_idx = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    // message to a group
                    self.ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?
                } else if let (Some(destination_uuid), Some(destination_e164)) = (
                    destination_uuid.and_then(|s| s.parse().ok()),
                    destination_e164,
                ) {
                    // message to a contact
                    self.ensure_contact_channel_exists(destination_uuid, &destination_e164)
                        .await
                } else {
                    return Ok(());
                };

                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let message = Message {
                    quote,
                    ..Message::new(self_uuid, text, timestamp)
                };
                (channel_idx, message)
            }
            // Incoming direct/group message
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
                    group_v2,
                    timestamp: Some(timestamp),
                    profile_key: Some(profile_key),
                    quote,
                    ..
                }),
            ) => {
                let (channel_idx, from) = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    // incoming group message
                    let channel_idx = self
                        .ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?;
                    let from = self
                        .ensure_user_is_known(uuid, profile_key, phone_number)
                        .await
                        .to_string();

                    (channel_idx, from)
                } else {
                    // incoming direct message
                    let name = self
                        .ensure_user_is_known(uuid, profile_key, phone_number)
                        .await
                        .to_string();
                    let channel_idx = self.ensure_contact_channel_exists(uuid, &name).await;
                    let from = self.data.channels.items[channel_idx].name.clone();

                    (channel_idx, from)
                };

                self.notify(&from, &text);

                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let message = Message {
                    quote,
                    ..Message::new(uuid, text, timestamp)
                };
                (channel_idx, message)
            }
            // reactions
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
                            destination_uuid,
                            message:
                                Some(DataMessage {
                                    body: None,
                                    group_v2,
                                    reaction:
                                        Some(Reaction {
                                            emoji: Some(emoji),
                                            remove,
                                            target_sent_timestamp: Some(target_sent_timestamp),
                                            ..
                                        }),
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) => {
                let channel_id = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    ..
                }) = group_v2
                {
                    ChannelId::Group(master_key)
                } else if let Some(uuid) = destination_uuid {
                    ChannelId::User(uuid.parse()?)
                } else {
                    return Ok(());
                };

                self.handle_reaction(
                    channel_id,
                    target_sent_timestamp,
                    sender_uuid,
                    remove.unwrap_or(false),
                    emoji,
                );
                return Ok(());
            }
            _ => return Ok(()),
        };

        self.add_message_to_channel(channel_idx, message);

        Ok(())
    }

    fn handle_reaction(
        &mut self,
        channel_id: ChannelId,
        target_sent_timestamp: u64,
        sender_uuid: Uuid,
        remove: bool,
        emoji: String,
    ) -> Option<()> {
        let channel_idx = self
            .data
            .channels
            .items
            .iter()
            .position(|channel| channel.id == channel_id)?;
        let channel = &mut self.data.channels.items[channel_idx];
        let message = channel
            .messages
            .items
            .iter_mut()
            .find(|m| m.arrived_at == target_sent_timestamp)?;
        let reaction_idx = message
            .reactions
            .iter()
            .position(|(from_id, _)| from_id == &sender_uuid);
        if let Some(idx) = reaction_idx {
            if remove {
                message.reactions.swap_remove(idx);
                self.save().unwrap();
            } else {
                message.reactions[idx].1 = emoji;
                self.touch_channel(channel_idx);
            }
        } else {
            message.reactions.push((sender_uuid, emoji));
            self.touch_channel(channel_idx);
        }
        Some(())
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
            .map(util::is_phone_number)
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
            .iter()
            .position(|channel| channel.user_id() == Some(uuid))
        {
            if let Some(name) = self.data.names.get(&uuid) {
                let channel = &mut self.data.channels.items[channel_idx];
                if &channel.name != name {
                    channel.name = name.clone();
                }
            }
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
        let channel = &mut self.data.channels.items[channel_idx];

        channel.messages.items.push(message);
        if let Some(idx) = channel.messages.state.selected() {
            // keep selection on the old message
            channel.messages.state.select(Some(idx + 1));
        }

        self.touch_channel(channel_idx);
    }

    fn touch_channel(&mut self, channel_idx: usize) {
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
