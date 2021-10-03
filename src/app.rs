use crate::config::Config;
use crate::signal::{
    self, GroupIdentifierBytes, GroupMasterKeyBytes, ResolvedGroup, SignalManager,
};
use crate::storage::Storage;
use crate::util::{self, LazyRegex, StatefulList, ATTACHMENT_REGEX, URL_REGEX};

use anyhow::{anyhow, Context as _};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use gh_emoji::Replacer;
use log::error;
use notify_rust::Notification;
use phonenumber::{Mode, PhoneNumber};
use presage::prelude::{
    content::{ContentBody, DataMessage, Metadata, ReceiptMessage, SyncMessage},
    proto::{
        data_message::{Quote, Reaction},
        sync_message::Sent,
        GroupContextV2,
    },
    AttachmentSpec, Content, GroupMasterKey, GroupSecretParams, ServiceAddress,
};
use regex_automata::Regex;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::fmt::Display;
use std::fs::File;
use std::path::Path;

pub struct App {
    pub config: Config,
    signal_manager: Box<dyn SignalManager>,
    storage: Box<dyn Storage>,
    pub user_id: Uuid,
    pub data: AppData,
    pub should_quit: bool,
    url_regex: LazyRegex,
    attachment_regex: LazyRegex,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonChannel")]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub group_data: Option<GroupData>,
    #[serde(serialize_with = "Channel::serialize_msgs")]
    pub messages: StatefulList<Message>,
    pub unread_messages: usize,
}

/// Proxy type which allows us to apply post-deserialization conversion.
///
/// Used to migrate the schema. Change this type only in backwards-compatible way.
#[derive(Deserialize)]
pub struct JsonChannel {
    pub id: ChannelId,
    pub name: String,
    #[serde(default)]
    pub group_data: Option<GroupData>,
    #[serde(deserialize_with = "Channel::deserialize_msgs")]
    pub messages: StatefulList<Message>,
    #[serde(default)]
    pub unread_messages: usize,
}

impl TryFrom<JsonChannel> for Channel {
    type Error = anyhow::Error;
    fn try_from(channel: JsonChannel) -> anyhow::Result<Self> {
        let mut channel = Channel {
            id: channel.id,
            name: channel.name,
            group_data: channel.group_data,
            messages: channel.messages,
            unread_messages: channel.unread_messages,
        };

        // 1. The master key in ChannelId::Group was replaced by group identifier,
        // the former was stored in group_data.
        match (channel.id, channel.group_data.as_mut()) {
            (ChannelId::Group(id), Some(group_data)) if group_data.master_key_bytes == [0; 32] => {
                group_data.master_key_bytes = id;
                channel.id = ChannelId::from_master_key_bytes(id)?;
            }
            _ => (),
        }
        Ok(channel)
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupData {
    #[serde(default)]
    pub master_key_bytes: GroupMasterKeyBytes,
    pub members: Vec<Uuid>,
    pub revision: u32,
}

impl Channel {
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

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChannelId {
    User(Uuid),
    Group(GroupIdentifierBytes),
}

impl From<Uuid> for ChannelId {
    fn from(id: Uuid) -> Self {
        ChannelId::User(id)
    }
}

impl ChannelId {
    fn from_master_key_bytes(bytes: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        let master_key_ar = bytes
            .as_ref()
            .try_into()
            .map_err(|_| anyhow!("invalid group master key"))?;
        let master_key = GroupMasterKey::new(master_key_ar);
        let secret_params = GroupSecretParams::derive_from_master_key(master_key);
        let group_id = secret_params.get_group_identifier();
        Ok(Self::Group(group_id))
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub enum BaseStatus {
    Sent,
    Received,
    Seen,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Status {
    Unknown,
    SingleStatusData(BaseStatus),
    GroupStatusData(Vec<(Uuid, BaseStatus)>),
}

impl BaseStatus {
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Sent => "X",
            Self::Received => "XX",
            Self::Seen => "XXX",
        }
    }
}

impl Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => f.write_str(""),
            Self::SingleStatusData(s) => f.write_str(s.to_str()),
            Self::GroupStatusData(v) => f.write_str(
                v.iter()
                    .map(|(_a, b)| b)
                    .min()
                    .unwrap_or(&BaseStatus::Sent)
                    .to_str(),
            ),
        }
    }
}

impl Default for Status {
    fn default() -> Self {
        Self::Unknown
    }
}

impl Status {
    pub fn new(is_group: bool) -> Self {
        if is_group {
            Self::GroupStatusData(Vec::new())
        } else {
            Self::SingleStatusData(BaseStatus::Sent)
        }
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
    #[serde(default)]
    pub status: Status,
}

impl Message {
    fn new(from_id: Uuid, message: String, arrived_at: u64, is_group: bool) -> Self {
        Self {
            from_id,
            message: Some(message),
            arrived_at,
            quote: None,
            attachments: Default::default(),
            reactions: Default::default(),
            status: Status::new(is_group),
        }
    }

    fn from_quote(quote: Quote, is_group: bool) -> Option<Message> {
        Some(Message {
            from_id: quote.author_uuid?.parse().ok()?,
            message: quote.text,
            arrived_at: quote.id?,
            quote: None,
            attachments: Default::default(),
            reactions: Default::default(),
            status: Status::new(is_group),
        })
    }

    pub fn update_status(&mut self, uuid: Uuid, new_status: BaseStatus) -> Option<()> {
        let debug = self.status.clone();
        match self.status {
            Status::Unknown => {
                log::warn!("Could not update status of message");
            }
            Status::SingleStatusData(ref mut s) => {
                assert!(new_status >= *s);
                *s = new_status
            }
            Status::GroupStatusData(ref mut v) => match v.iter_mut().find(|(u, _s)| *u == uuid) {
                Some((_u, _s)) => {
                    assert!(new_status >= *_s);
                    *_s = new_status;
                }
                None => v.push((uuid, new_status)),
            },
        }
        log::error!(
            "Updated message status from {:?} to {:?}",
            debug,
            self.status
        );
        Some(())
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
    pub fn try_new(
        config: Config,
        signal_manager: Box<dyn SignalManager>,
        storage: Box<dyn Storage>,
    ) -> anyhow::Result<Self> {
        let user_id = signal_manager.user_id();
        let data = storage.load_app_data(user_id, config.user.name.clone())?;
        Ok(Self {
            config,
            signal_manager,
            storage,
            user_id,
            data,
            should_quit: false,
            url_regex: LazyRegex::new(URL_REGEX),
            attachment_regex: LazyRegex::new(ATTACHMENT_REGEX),
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.storage.save_app_data(&self.data)
    }

    pub fn name_by_id(&self, id: Uuid) -> &str {
        name_by_id(&self.data.names, id)
    }

    pub fn put_char(&mut self, c: char) {
        let idx = self.data.input_cursor;
        self.data.input.insert(idx, c);
        self.data.input_cursor += c.len_utf8();
        self.data.input_cursor_chars += 1;
    }

    pub fn on_key(&mut self, key: KeyCode) -> anyhow::Result<()> {
        match key {
            KeyCode::Char('\r') => self.put_char('\n'),
            KeyCode::Enter if !self.data.input.is_empty() => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.send_input(idx)?;
                }
            }
            KeyCode::Enter => {
                // input is empty
                self.try_open_url();
            }
            KeyCode::Home => self.on_home(),
            KeyCode::End => self.on_end(),
            KeyCode::Backspace => {
                self.on_backspace();
            }
            KeyCode::Esc => self.reset_message_selection(),
            KeyCode::Char(c) => self.put_char(c),
            KeyCode::Tab => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.add_reaction(idx, self.data.input.is_empty()).await;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn add_reaction(&mut self, channel_idx: usize, remove: bool) -> Option<()> {
        let input: String = self.data.input.drain(..).collect();
        let reaction = Self::take_emoji(&input);
        let message_reaction = reaction.clone();

        self.data.input_cursor = 0;
        self.data.input_cursor_chars = 0;

        if let Some(selected_message) = self.data.channels.items[channel_idx].selected_message() {
            let timestamp = util::utc_now_timestamp_msec();
            let target_author_uuid = selected_message.from_id;
            let target_sent_timestamp = selected_message.arrived_at;
            let mut data_message = DataMessage {
                body: None,
                quote: None,
                reaction: Some(Reaction {
                    emoji: if !remove {
                        Some((reaction.to_owned())?)
                    } else {
                        Some(
                            selected_message
                                .reactions
                                .iter()
                                .find(|(uuid, _reac)| *uuid == self.self_id())?
                                .1
                                .clone(),
                        )
                    },
                    remove: Some(remove),
                    target_author_uuid: Some(target_author_uuid.to_string()),
                    target_sent_timestamp: Some(target_sent_timestamp),
                }),
                ..Default::default()
            };
            match self.data.channels.items[channel_idx].id {
                ChannelId::User(uuid) => {
                    let manager = self.signal_manager.clone();
                    let body = ContentBody::DataMessage(data_message);
                    tokio::task::spawn_local(async move {
                        if let Err(e) = manager.send_message(uuid, body, timestamp).await {
                            log::error!(
                                "Failed to send reaction {:?} to {}: {}",
                                &message_reaction,
                                uuid,
                                e
                            );
                            return;
                        }
                    });
                }
                ChannelId::Group(_) => {
                    if let Some(group_data) =
                        self.data.channels.items[channel_idx].group_data.as_ref()
                    {
                        let manager = self.signal_manager.clone();
                        let self_uuid = self.signal_manager.uuid();

                        data_message.group_v2 = Some(GroupContextV2 {
                            master_key: Some(group_data.master_key_bytes.to_vec()),
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
                                log::error!(
                                    "Failed to send group reaction {:?} : {}",
                                    &message_reaction,
                                    e
                                );
                                return;
                            }
                        });
                    } else {
                        error!("cannot send to broken channel without group data");
                    }
                }
            }
            if remove || reaction.is_some() {
                self.handle_reaction(
                    self.data.channels.items[channel_idx].id,
                    target_sent_timestamp,
                    target_author_uuid,
                    remove,
                    reaction.unwrap_or_default(),
                );
            }
        }

        self.reset_unread_messages();
        self.bubble_up_channel(channel_idx);
        self.save().unwrap();
        self.reset_message_selection();
        Some(())
    }

    /// Returns the emoji and leaves the `input` empty if it is of the shape `:some_real_emoji`.
    fn take_emoji(input: &str) -> Option<String> {
        let s = input.trim().to_owned();
        if emoji::lookup_by_glyph::lookup(s.as_str()).is_some() {
            Some(s)
        } else {
            let s = s.strip_prefix(':')?.strip_suffix(':')?;
            let emoji = gh_emoji::get(s)?.to_string();
            Some(emoji)
        }
    }

    fn reset_message_selection(&mut self) {
        if let Some(idx) = self.data.channels.state.selected() {
            let channel = &mut self.data.channels.items[idx];
            channel.messages.state.select(None);
            channel.messages.rendered = Default::default();
        }
    }

    async fn send_input(&mut self, channel_idx: usize) {
        let emoji_replacer = Replacer::new();
        let channel = &mut self.data.channels.items[channel_idx];

        let message: String = emoji_replacer
            .replace_all(&(self.data.input.drain(..).collect::<String>()))
            .into_owned();
        self.data.input_cursor = 0;
        self.data.input_cursor_chars = 0;
        std::mem::take(&mut self.data.input)
    }

    fn send_input(&mut self, channel_idx: usize) -> anyhow::Result<()> {
        let input = self.take_input();
        let (input, attachments) = self.extract_attachments(&input);
        let channel = &mut self.data.channels.items[channel_idx];
        let quote = channel.selected_message();
        let sent_message = self
            .signal_manager
            .send_text(channel, input, quote, attachments);

        let quote = channel.selected_message().map(|message| Quote {
            // Messages are shown in reverse order => selected is reverse
            id: Some(message.arrived_at),
            author_uuid: Some(message.from_id.to_string()),
            text: message.message.clone(),
            ..Default::default()
        });

        let quote_message = quote
            .clone()
            .and_then(|q| Message::from_quote(q, channel.group_data.is_some()))
            .map(Box::new);
        let with_quote = quote.is_some();

        let mut data_message = DataMessage {
            body: Some(message.clone()),
            timestamp: Some(timestamp),
            quote,
            ..Default::default()
        };

        let mut is_group = false;

        match channel.id {
            ChannelId::User(uuid) => {
                let manager = self.signal_manager.clone();
                let body = ContentBody::DataMessage(data_message);
                tokio::task::spawn_local(async move {
                    if let Err(e) = manager.send_message(uuid, body, timestamp).await {
                        // TODO: Proper error handling
                        log::error!("Failed to send message to {}: {}", uuid, e);
                    }
                });
            }
            ChannelId::Group(_) => {
                is_group = true;
                if let Some(group_data) = channel.group_data.as_ref() {
                    let manager = self.signal_manager.clone();
                    let self_uuid = self.signal_manager.uuid();

                    data_message.group_v2 = Some(GroupContextV2 {
                        master_key: Some(group_data.master_key_bytes.to_vec()),
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
            status: if is_group {
                Status::GroupStatusData(Vec::new())
            } else {
                Status::SingleStatusData(BaseStatus::Sent)
            },
        });

        self.reset_unread_messages();
        if sent_with_quote {
            self.reset_message_selection();
        }
        self.bubble_up_channel(channel_idx);
        self.save()
    }

    pub fn select_previous_channel(&mut self) {
        if self.reset_unread_messages() {
            self.save().unwrap();
        }
        self.data.channels.previous();
    }

    pub fn select_next_channel(&mut self) {
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
    pub fn move_back_word(&mut self) {
        self.on_left();
        self.word_operation(Self::on_left);
        if self.data.input.as_bytes().get(self.data.input_cursor) == Some(&b' ') {
            self.on_right();
        }
    }

    /// Move a word forward
    pub fn move_forward_word(&mut self) {
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

        let user_id = self.user_id;

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
            ) if destination_uuid.parse() == Ok(user_id) => {
                let channel_idx = self.ensure_own_channel_exists();
                let message = Message::new(self_uuid, text, timestamp, false);
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
                let (channel_idx, is_group) = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    // message to a group
                    let master_key = master_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid master key"))?;
                    (
                        self.ensure_group_channel_exists(master_key, revision)
                            .await
                            .context("failed to create group channel")?,
                        true,
                    )
                } else if let (Some(destination_uuid), Some(destination_e164)) = (
                    destination_uuid.and_then(|s| s.parse().ok()),
                    destination_e164,
                ) {
                    // message to a contact
                    (
                        self.ensure_contact_channel_exists(destination_uuid, &destination_e164)
                            .await,
                        false,
                    )
                } else {
                    return Ok(());
                };

                let quote = quote
                    .and_then(|q| Message::from_quote(q, is_group))
                    .map(Box::new);
                let message = Message {
                    quote,
                    ..Message::new(self_uuid, text, timestamp, is_group)
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
                let (channel_idx, from, is_group) = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    // incoming group message
                    let master_key = master_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid group master key"))?;
                    let channel_idx = self
                        .ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?;
                    let from = self
                        .ensure_user_is_known(uuid, profile_key, phone_number)
                        .await
                        .to_string();

                    (channel_idx, from, true)
                } else {
                    // incoming direct message
                    let name = self
                        .ensure_user_is_known(uuid, profile_key, phone_number)
                        .await
                        .to_string();
                    let channel_idx = self.ensure_contact_channel_exists(uuid, &name).await;
                    let from = self.data.channels.items[channel_idx].name.clone();

                    (channel_idx, from, false)
                };

                self.notify(&from, &text);

                let quote = quote
                    .and_then(|q| Message::from_quote(q, is_group))
                    .map(Box::new);
                let message = Message {
                    quote,
                    ..Message::new(uuid, text, timestamp, is_group)
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
                                            target_author_uuid: Some(target_author_uuid),
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
                    ChannelId::from_master_key_bytes(master_key)?
                } else if let Some(uuid) = destination_uuid {
                    ChannelId::User(uuid.parse()?)
                } else {
                    ChannelId::User(target_author_uuid.parse()?)
                };

                self.handle_reaction(
                    channel_id,
                    target_sent_timestamp,
                    sender_uuid,
                    emoji,
                    remove.unwrap_or(false),
                    true,
                );
                return Ok(());
            }
            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: Some(sender_uuid),
                            ..
                        },
                    ..
                },
                ContentBody::DataMessage(DataMessage {
                    body: None,
                    group_v2,
                    reaction:
                        Some(Reaction {
                            emoji: Some(emoji),
                            remove,
                            target_sent_timestamp: Some(target_sent_timestamp),
                            target_author_uuid: Some(target_author_uuid),
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
                    ChannelId::from_master_key_bytes(master_key)?
                } else if sender_uuid == self.user_id {
                    // reaction from us => target author is the user channel
                    ChannelId::User(target_author_uuid.parse()?)
                } else {
                    // reaction is from somebody else => they are the user channel
                    ChannelId::User(sender_uuid)
                };

                self.handle_reaction(
                    channel_id,
                    target_sent_timestamp,
                    sender_uuid,
                    emoji,
                    remove.unwrap_or(false),
                    true,
                );
                return Ok(());
            }
            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: Some(sender_uuid),
                            ..
                        },
                    ..
                },
                ContentBody::ReceiptMessage(ReceiptMessage { r#type, timestamp }),
            ) => {
                for ts in timestamp.iter() {
                    let message = self.find_message_by_timestamp(ts);
                    log::error!("Received status {:?} on timestamp {}", r#type, ts);
                    if let Some(m) = message {
                        log::error!("Found message for status");
                        let new_status = match r#type {
                            Some(0) => BaseStatus::Received,
                            Some(1) => BaseStatus::Seen,
                            _ => {
                                log::error!("Could not discover type of status in message from {} on timestamped message {:?} : {:?}", sender_uuid, timestamp, r#type);
                                return Ok(());
                            }
                        };
                        m.update_status(sender_uuid, new_status);
                    }
                }
                return Ok(());
            }
            _ => return Ok(()),
        };

        self.add_message_to_channel(channel_idx, message);

        Ok(())
    }

    fn find_message_by_timestamp(&mut self, timestamp: &u64) -> Option<&mut Message> {
        let channel = self
            .data
            .channels
            .items
            .iter_mut()
            .find(|c| c.messages.items.iter().any(|m| m.arrived_at == *timestamp))?;
        let message = channel
            .messages
            .items
            .iter_mut()
            .find(|m| m.arrived_at == *timestamp)?;
        Some(message)
    }

    fn handle_reaction(
        &mut self,
        channel_id: ChannelId,
        target_sent_timestamp: u64,
        sender_uuid: Uuid,
        emoji: String,
        remove: bool,
        notify: bool,
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
        let is_added = if let Some(idx) = reaction_idx {
            if remove {
                message.reactions.swap_remove(idx);
                false
            } else {
                message.reactions[idx].1 = emoji.clone();
                true
            }
        } else {
            message.reactions.push((sender_uuid, emoji.clone()));
            true
        };

        if is_added && channel_id != ChannelId::User(self.user_id) {
            // Notification
            let sender_name = name_by_id(&self.data.names, sender_uuid);
            let summary = if let ChannelId::Group(_) = channel.id {
                Cow::from(format!("{} in {}", sender_name, channel.name))
            } else {
                Cow::from(sender_name)
            };
            let mut notification = format!("{} reacted {}", summary, emoji);
            if let Some(text) = message.message.as_ref() {
                notification.push_str(" to: ");
                notification.push_str(text);
            }
            if notify {
                self.notify(&summary, &notification);
            }

            self.touch_channel(channel_idx);
        } else {
            self.save().unwrap();
        }

        Some(())
    }

    async fn ensure_group_channel_exists(
        &mut self,
        master_key: GroupMasterKeyBytes,
        revision: u32,
    ) -> anyhow::Result<usize> {
        let id = ChannelId::from_master_key_bytes(master_key)?;
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
                let ResolvedGroup {
                    name,
                    group_data,
                    profile_keys,
                } = self.signal_manager.resolve_group(master_key).await?;

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
            let ResolvedGroup {
                name,
                group_data,
                profile_keys,
            } = self.signal_manager.resolve_group(master_key).await?;

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
        phone_number: PhoneNumber,
    ) -> &str {
        if self
            .try_ensure_user_is_known(uuid, profile_key)
            .await
            .is_none()
        {
            let phone_number_name = phone_number.format().mode(Mode::E164).to_string();
            self.data.names.insert(uuid, phone_number_name);
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
                Ok(key) => self.signal_manager.contact_name(uuid, key).await,
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
        let user_id = self.user_id;
        if let Some(channel_idx) = self
            .data
            .channels
            .items
            .iter_mut()
            .position(|channel| channel.user_id() == Some(user_id))
        {
            channel_idx
        } else {
            self.data.channels.items.push(Channel {
                id: user_id.into(),
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
        if let Err(e) = Notification::new().summary(summary).body(text).show() {
            error!("failed to send notification: {}", e);
        }
    }

    fn extract_attachments(&mut self, input: &str) -> (String, Vec<(AttachmentSpec, Vec<u8>)>) {
        let mut offset = 0;
        let mut clean_input = String::new();

        let re = self.attachment_regex.compiled();
        let attachments = re.find_iter(input.as_bytes()).filter_map(|(start, end)| {
            let path_str = &input[start..end].strip_prefix("file://")?;

            let path = Path::new(path_str);
            let contents = std::fs::read(path).ok()?;

            clean_input.push_str(input[offset..start].trim_end_matches(""));
            offset = end;

            let content_type = mime_guess::from_path(path)
                .first()
                .map(|mime| mime.essence_str().to_string())
                .unwrap_or_default();
            let spec = AttachmentSpec {
                content_type,
                length: contents.len(),
                file_name: Path::new(path)
                    .file_name()
                    .map(|f| f.to_string_lossy().into()),
                preview: None,
                voice_note: None,
                borderless: None,
                width: None,
                height: None,
                caption: None,
                blur_hash: None,
            };
            Some((spec, contents))
        });

        let attachments = attachments.collect();
        clean_input.push_str(&input[offset..]);
        let clean_input = clean_input.trim().to_string();

        (clean_input, attachments)
    }
}

pub fn name_by_id(names: &HashMap<Uuid, String>, id: Uuid) -> &str {
    names.get(&id).map(|s| s.as_ref()).unwrap_or("Unknown Name")
}

/// Returns an emoji string if `s` is an emoji or if `s` is a GitHub emoji shortcode.
fn to_emoji(s: &str) -> Option<&str> {
    let s = s.trim();
    if emoji::lookup_by_glyph::lookup(s).is_some() {
        Some(s)
    } else {
        let s = s.strip_prefix(':')?.strip_suffix(':')?;
        let emoji = gh_emoji::get(s)?;
        Some(emoji)
    }
}

fn open_url(message: &Message, url_regex: &Regex) -> Option<()> {
    let text = message.message.as_ref()?;
    let (start, end) = url_regex.find(text.as_bytes())?;
    let url = &text[start..end];
    if let Err(e) = opener::open(url) {
        error!("failed to open {}: {}", url, e);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::User;
    use crate::signal::test::SignalManagerMock;
    use crate::storage::test::InMemoryStorage;

    use std::cell::RefCell;
    use std::rc::Rc;

    fn test_app() -> (App, Rc<RefCell<Vec<Message>>>) {
        let signal_manager = SignalManagerMock::new();
        let sent_messages = signal_manager.sent_messages.clone();

        let mut app = App::try_new(
            Config::with_user(User {
                name: "Tyler Durden".to_string(),
                phone_number: "+0000000000".to_string(),
            }),
            Box::new(signal_manager),
            Box::new(InMemoryStorage::new()),
        )
        .unwrap();

        app.data.channels.items.push(Channel {
            id: ChannelId::User(Uuid::new_v4()),
            name: "test".to_string(),
            group_data: Some(GroupData {
                master_key_bytes: GroupMasterKeyBytes::default(),
                members: vec![app.user_id],
                revision: 1,
            }),
            messages: StatefulList::with_items(vec![Message {
                from_id: app.user_id,
                message: Some("First message".to_string()),
                arrived_at: 0,
                quote: Default::default(),
                attachments: Default::default(),
                reactions: Default::default(),
            }]),
            unread_messages: 1,
        });
        app.data.channels.state.select(Some(0));

        (app, sent_messages)
    }

    #[test]
    fn test_send_input() {
        let (mut app, sent_messages) = test_app();
        let input = "Hello, World!";
        for c in input.chars() {
            app.put_char(c);
        }
        app.send_input(0).unwrap();

        let sent = sent_messages.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].message.as_ref().unwrap(), input);

        assert_eq!(app.data.channels.items[0].unread_messages, 0);

        assert_eq!(app.data.input, "");
        assert_eq!(app.data.input_cursor, 0);
        assert_eq!(app.data.input_cursor_chars, 0);
    }

    #[test]
    fn test_send_input_with_emoji() {
        let (mut app, sent_messages) = test_app();
        let input = "👻";
        for c in input.chars() {
            app.put_char(c);
        }
        assert_eq!(app.data.input_cursor, 4);
        assert_eq!(app.data.input_cursor_chars, 1);

        app.send_input(0).unwrap();

        let sent = sent_messages.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].message.as_ref().unwrap(), input);

        assert_eq!(app.data.input, "");
        assert_eq!(app.data.input_cursor, 0);
        assert_eq!(app.data.input_cursor_chars, 0);
    }

    #[test]
    fn test_send_input_with_emoji_codepoint() {
        let (mut app, sent_messages) = test_app();
        let input = ":thumbsup:";
        for c in input.chars() {
            app.put_char(c);
        }

        app.send_input(0).unwrap();

        let sent = sent_messages.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].message.as_ref().unwrap(), "👍");
    }

    #[test]
    fn test_add_reaction_with_emoji() {
        let (mut app, _sent_messages) = test_app();

        app.data.channels.items[0].messages.state.select(Some(0));

        app.put_char('👍');
        app.add_reaction(0);

        let reactions = &app.data.channels.items[0].messages.items[0].reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0], (app.user_id, "👍".to_string()));
    }

    #[test]
    fn test_add_reaction_with_emoji_codepoint() {
        let (mut app, _sent_messages) = test_app();

        app.data.channels.items[0].messages.state.select(Some(0));

        for c in ":thumbsup:".chars() {
            app.put_char(c);
        }
        app.add_reaction(0);

        let reactions = &app.data.channels.items[0].messages.items[0].reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0], (app.user_id, "👍".to_string()));
    }

    #[test]
    fn test_remove_reaction() {
        let (mut app, _sent_messages) = test_app();

        app.data.channels.items[0].messages.state.select(Some(0));
        let reactions = &mut app.data.channels.items[0].messages.items[0].reactions;
        reactions.push((app.user_id, "👍".to_string()));

        app.add_reaction(0);

        let reactions = &app.data.channels.items[0].messages.items[0].reactions;
        assert!(reactions.is_empty());
    }

    #[test]
    fn test_add_invalid_reaction() {
        let (mut app, _sent_messages) = test_app();

        app.data.channels.items[0].messages.state.select(Some(0));

        for c in ":thumbsup".chars() {
            app.put_char(c);
        }
        app.add_reaction(0);

        assert_eq!(app.data.input, ":thumbsup");
        let reactions = &app.data.channels.items[0].messages.items[0].reactions;
        assert!(reactions.is_empty());
    }
}
