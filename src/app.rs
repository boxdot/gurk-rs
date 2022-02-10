use crate::config::Config;
use crate::cursor::Cursor;
use crate::signal::{
    self, Attachment, GroupIdentifierBytes, GroupMasterKeyBytes, ResolvedGroup, SignalManager,
};
use crate::storage::Storage;
use crate::util::{
    self, FilteredStatefulList, LazyRegex, StatefulList, ATTACHMENT_REGEX, URL_REGEX,
};

use anyhow::{anyhow, Context as _};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use itertools::FoldWhile::{Continue, Done};
use itertools::Itertools;
use notify_rust::Notification;
use phonenumber::{Mode, PhoneNumber};
use presage::prelude::proto::{AttachmentPointer, ReceiptMessage, TypingMessage};
use presage::prelude::{
    content::{ContentBody, DataMessage, Metadata, SyncMessage},
    proto::{
        data_message::{Quote, Reaction},
        sync_message::Sent,
        GroupContextV2,
    },
    AttachmentSpec, Content, GroupMasterKey, GroupSecretParams, ServiceAddress,
};
use regex_automata::Regex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::borrow::Cow;
use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::convert::{TryFrom, TryInto};
use std::path::Path;
use std::str::FromStr;

pub struct App {
    pub config: Config,
    signal_manager: Box<dyn SignalManager>,
    storage: Box<dyn Storage>,
    pub user_id: Uuid,
    pub data: AppData,
    pub should_quit: bool,
    url_regex: LazyRegex,
    attachment_regex: LazyRegex,
    display_help: bool,
    pub is_searching: bool,
    pub channel_text_width: usize,
    receipt_handler: ReceiptHandler,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReceiptHandler {
    receipt_set: HashMap<Uuid, ReceiptQueues>,
    time_since_update: u64,
}

impl ReceiptHandler {
    pub fn new() -> Self {
        Self {
            receipt_set: HashMap::new(),
            time_since_update: 0u64,
        }
    }

    pub fn add_receipt_event(&mut self, event: ReceiptEvent) {
        // Add a new set in the case no receipt had been handled for this contact
        // over the current session
        self.receipt_set
            .entry(event.uuid)
            .or_insert_with(ReceiptQueues::new)
            .add(event.timestamp, event.receipt_type);
    }

    // Dictates whether receipts should be sent on the current tick
    // Not used for now as
    fn do_tick(&mut self) -> bool {
        true
    }

    pub fn step(&mut self, signal_manager: &dyn SignalManager) -> bool {
        if !self.do_tick() {
            return false;
        }
        if self.receipt_set.is_empty() {
            return false;
        }

        // Get any key
        let uuid = *self.receipt_set.keys().next().unwrap();

        let j = self.receipt_set.entry(uuid);
        match j {
            Entry::Occupied(mut e) => {
                let u = e.get_mut();
                if let Some((timestamps, receipt)) = u.get_data() {
                    signal_manager.send_receipt(uuid, timestamps, receipt);
                    if u.is_empty() {
                        e.remove_entry();
                    }
                    true
                } else {
                    false
                }
            }
            Entry::Vacant(_) => false,
        }
    }
}

/// This get built anywhere in the client and get passed to the App
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReceiptEvent {
    uuid: Uuid,
    /// Timestamp of the messages
    timestamp: u64,
    /// Type : Received, Delivered
    receipt_type: Receipt,
}

impl ReceiptEvent {
    pub fn new(uuid: Uuid, timestamp: u64, receipt_type: Receipt) -> Self {
        Self {
            uuid,
            timestamp,
            receipt_type,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReceiptQueues {
    received_msg: HashSet<u64>,
    read_msg: HashSet<u64>,
}

impl ReceiptQueues {
    pub fn new() -> Self {
        Self {
            received_msg: HashSet::new(),
            read_msg: HashSet::new(),
        }
    }

    pub fn add_received(&mut self, timestamp: u64) {
        if !self.received_msg.insert(timestamp) {
            log::error!("Somehow got duplicate Received receipt @ {}", timestamp);
        }
    }

    pub fn add_read(&mut self, timestamp: u64) {
        // Ensures we do not send uselessly double the amount of receipts
        // in the case a message is immediatly received and read.
        self.received_msg.remove(&timestamp);
        if !self.read_msg.insert(timestamp) {
            log::error!("Somehow got duplicate Delivered receipt @ {}", timestamp);
        }
    }

    pub fn add(&mut self, timestamp: u64, receipt: Receipt) {
        match receipt {
            Receipt::Received => self.add_received(timestamp),
            Receipt::Delivered => self.add_read(timestamp),
            _ => {}
        }
    }

    pub fn get_data(&mut self) -> Option<(Vec<u64>, Receipt)> {
        if !self.received_msg.is_empty() {
            let timestamps = self.received_msg.drain().collect::<Vec<u64>>();
            return Some((timestamps, Receipt::Received));
        }
        if !self.read_msg.is_empty() {
            let timestamps = self.read_msg.drain().collect::<Vec<u64>>();
            return Some((timestamps, Receipt::Delivered));
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.received_msg.is_empty() && self.read_msg.is_empty()
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BoxData {
    pub data: String,
    pub cursor: Cursor,
}

impl BoxData {
    #[cfg(test)]
    pub fn empty() -> Self {
        Default::default()
    }

    pub fn put_char(&mut self, c: char) {
        self.cursor.put(c, &mut self.data);
    }

    pub fn new_line(&mut self) {
        self.cursor.new_line(&mut self.data);
    }

    pub fn on_left(&mut self) {
        self.cursor.move_left(&self.data);
    }

    pub fn on_right(&mut self) {
        self.cursor.move_right(&self.data);
    }

    pub fn move_line_down(&mut self) {
        self.cursor.move_line_down(&self.data);
    }

    pub fn move_line_up(&mut self) {
        self.cursor.move_line_up(&self.data);
    }

    pub fn move_back_word(&mut self) {
        self.cursor.move_word_left(&self.data);
    }

    pub fn move_forward_word(&mut self) {
        self.cursor.move_word_right(&self.data);
    }

    pub fn on_home(&mut self) {
        self.cursor.start_of_line(&self.data);
    }

    pub fn on_end(&mut self) {
        self.cursor.end_of_line(&self.data);
    }

    pub fn on_backspace(&mut self) {
        self.cursor.delete_backward(&mut self.data);
    }

    pub fn on_delete_word(&mut self) {
        self.cursor.delete_word_backward(&mut self.data);
    }

    pub fn on_delete_suffix(&mut self) {
        self.cursor.delete_suffix(&mut self.data);
    }

    fn take(&mut self) -> String {
        self.cursor = Default::default();
        std::mem::take(&mut self.data)
    }
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppData {
    pub channels: FilteredStatefulList<Channel>,
    pub names: HashMap<Uuid, String>,
    #[serde(skip)] // ! We may want to save it
    pub input: BoxData,
    #[serde(skip)]
    pub search_box: BoxData,
    #[serde(skip)]
    pub is_multiline_input: bool,
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
    pub typing: TypingSet,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypingSet {
    SingleTyping(bool),
    GroupTyping(HashSet<Uuid>),
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
        let is_group = channel.group_data.is_some();
        let mut channel = Channel {
            id: channel.id,
            name: channel.name,
            group_data: channel.group_data,
            messages: channel.messages,
            unread_messages: channel.unread_messages,
            typing: {
                if is_group {
                    TypingSet::GroupTyping(HashSet::new())
                } else {
                    TypingSet::SingleTyping(false)
                }
            },
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
    pub fn contains_user(&self, name: &str, hm: &HashMap<Uuid, String>) -> bool {
        match self.group_data {
            Some(ref gd) => gd.members.iter().any(|u| name_by_id(hm, *u).contains(name)),
            None => self.name.contains(name),
        }
    }

    pub fn match_pattern(&self, pattern: &str, hm: &HashMap<Uuid, String>) -> bool {
        if pattern.is_empty() {
            return true;
        }
        match pattern.chars().next().unwrap() {
            '@' => self.contains_user(&pattern[1..], hm),
            _ => self.name.contains(pattern),
        }
    }

    pub fn reset_writing(&mut self, user: Uuid) {
        match &mut self.typing {
            TypingSet::GroupTyping(ref mut hash_set) => {
                hash_set.remove(&user);
            }
            TypingSet::SingleTyping(_) => {
                self.typing = TypingSet::SingleTyping(false);
            }
        }
    }

    pub fn is_writing(&self) -> bool {
        match &self.typing {
            TypingSet::GroupTyping(a) => !a.is_empty(),
            TypingSet::SingleTyping(a) => *a,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TypingAction {
    Started,
    Stopped,
}

impl TypingAction {
    pub fn from_i32(i: i32) -> Self {
        match i {
            0 => Self::Started,
            1 => Self::Stopped,
            _ => {
                log::error!("Got incorrect TypingAction : {}", i);
                Self::Stopped
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Receipt {
    Nothing, // Do not do anything to these receipts in order to avoid spamming receipt messages when an old database is loaded
    Sent,
    Received,
    Delivered,
}

impl Default for Receipt {
    fn default() -> Self {
        Self::Nothing
    }
}

impl Receipt {
    pub fn write(&self) -> &'static str {
        match self {
            Self::Nothing => "",
            Self::Sent => "(x)",
            Self::Received => "(xx)",
            Self::Delivered => "(xxx)",
        }
    }

    pub fn update(&self, other: Self) -> Self {
        *self.max(&other)
    }

    pub fn from_i32(i: i32) -> Self {
        match i {
            0 => Self::Received,
            1 => Self::Delivered,
            _ => Self::Nothing,
        }
    }

    pub fn to_i32(self) -> i32 {
        match self {
            Self::Delivered => 1,
            _ => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub receipt: Receipt,
}

impl Message {
    fn new(
        from_id: Uuid,
        message: Option<String>,
        arrived_at: u64,
        attachments: Vec<Attachment>,
    ) -> Self {
        Self {
            from_id,
            message,
            arrived_at,
            quote: None,
            attachments,
            reactions: Default::default(),
            receipt: Receipt::Sent,
        }
    }

    pub fn from_quote(quote: Quote) -> Option<Message> {
        Some(Message {
            from_id: quote.author_uuid?.parse().ok()?,
            message: quote.text,
            arrived_at: quote.id?,
            quote: None,
            attachments: Default::default(),
            reactions: Default::default(),
            receipt: Receipt::Sent,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.message.is_none() && self.attachments.is_empty() && self.reactions.is_empty()
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
    Tick,
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
            display_help: false,
            is_searching: false,
            channel_text_width: 0,
            receipt_handler: ReceiptHandler::new(),
        })
    }

    pub fn get_input(&mut self) -> &mut BoxData {
        if self.is_searching {
            &mut self.data.search_box
        } else {
            &mut self.data.input
        }
    }

    pub fn writing_people(&self, channel: &Channel) -> String {
        if !channel.is_writing() {
            return String::from("");
        }
        let uuids: Vec<Uuid> = match &channel.typing {
            TypingSet::GroupTyping(hash_set) => hash_set.clone().into_iter().collect(),
            TypingSet::SingleTyping(a) => {
                if *a {
                    vec![channel.user_id().unwrap()]
                } else {
                    Vec::new()
                }
            }
        };
        format!(
            "{:?} writing...",
            uuids
                .into_iter()
                .map(|u| self.name_by_id(u))
                .collect::<Vec<&str>>()
        )
    }

    pub fn save(&self) -> anyhow::Result<()> {
        self.storage.save_app_data(&self.data)
    }

    pub fn name_by_id(&self, id: Uuid) -> &str {
        name_by_id(&self.data.names, id)
    }

    pub fn on_key(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        match key.code {
            KeyCode::Char('\r') => self.get_input().put_char('\n'),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) && !self.is_searching => {
                self.data.is_multiline_input = !self.data.is_multiline_input;
            }
            KeyCode::Enter if self.data.is_multiline_input && !self.is_searching => {
                self.get_input().new_line();
            }
            KeyCode::Enter if !self.get_input().data.is_empty() && !self.is_searching => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.send_input(self.data.channels.filtered_items[idx])?;
                }
            }
            KeyCode::Enter => {
                // input is empty
                self.try_open_url();
            }
            KeyCode::Home => {
                self.get_input().on_home();
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.get_input().on_home();
            }
            KeyCode::End => {
                self.get_input().on_end();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.get_input().on_end();
            }
            KeyCode::Backspace => {
                self.get_input().on_backspace();
            }
            KeyCode::Esc => self.reset_message_selection(),
            KeyCode::Char(c) => self.get_input().put_char(c),
            KeyCode::Tab => {
                if let Some(idx) = self.data.channels.state.selected() {
                    self.add_reaction(idx);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Tries to open the first url in the selected message.
    ///
    /// Does nothing if no message is selected and no url is contained in the message.
    fn try_open_url(&mut self) -> Option<()> {
        let channel_idx = self.data.channels.state.selected()?;
        let channel = &self.data.channels.items[channel_idx];
        let message = channel.selected_message()?;
        let re = self.url_regex.compiled();
        open_url(message, re)?;
        self.reset_message_selection();
        Some(())
    }

    /// Returns Some(_) reaction if input is a reaction.
    ///
    /// Inner is None, if the reaction should be removed.
    fn take_reaction(&mut self) -> Option<Option<String>> {
        let input_box = self.get_input();
        if input_box.data.is_empty() {
            Some(None)
        } else {
            let emoji = to_emoji(&input_box.data)?.to_string();
            self.take_input();
            Some(Some(emoji))
        }
    }

    pub fn add_reaction(&mut self, channel_idx: usize) -> Option<()> {
        let reaction = self.take_reaction()?;
        let channel = &self.data.channels.items[channel_idx];
        let message = channel.selected_message()?;
        let remove = reaction.is_none();
        let emoji = reaction.or_else(|| {
            // find emoji which should be removed
            // if no emoji found => there is no reaction from us => nothing to remove
            message.reactions.iter().find_map(|(id, emoji)| {
                if id == &self.signal_manager.user_id() {
                    Some(emoji.clone())
                } else {
                    None
                }
            })
        })?;

        self.signal_manager
            .send_reaction(channel, message, emoji.clone(), remove);

        let channel_id = channel.id;
        let arrived_at = message.arrived_at;
        self.handle_reaction(
            channel_id,
            arrived_at,
            self.signal_manager.user_id(),
            emoji,
            remove,
            false,
        );

        self.reset_unread_messages();
        self.bubble_up_channel(channel_idx);
        self.reset_message_selection();

        self.save().unwrap();
        Some(())
    }

    fn reset_message_selection(&mut self) {
        if let Some(idx) = self.data.channels.state.selected() {
            let channel = &mut self.data.channels.items[idx];
            channel.messages.state.select(None);
            channel.messages.rendered = Default::default();
        }
    }

    fn take_input(&mut self) -> String {
        self.get_input().take()
    }

    fn send_input(&mut self, channel_idx: usize) -> anyhow::Result<()> {
        let input = self.take_input();
        let (input, attachments) = self.extract_attachments(&input);
        let channel = &mut self.data.channels.items[channel_idx];
        let quote = channel.selected_message();
        let sent_message = self
            .signal_manager
            .send_text(channel, input, quote, attachments);

        let sent_with_quote = sent_message.quote.is_some();
        channel.messages.items.push(sent_message);

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

    pub async fn on_message(&mut self, content: Content) -> anyhow::Result<()> {
        // log::debug!("incoming: {:#?}", content);
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
                                    body,
                                    attachments: attachment_pointers,
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if destination_uuid.parse() == Ok(user_id) => {
                let channel_idx = self.ensure_own_channel_exists();
                let attachments = self.save_attachments(attachment_pointers).await;
                let message = Message::new(user_id, body, timestamp, attachments);
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
                                    body,
                                    group_v2,
                                    quote,
                                    attachments: attachment_pointers,
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if sender_uuid == user_id => {
                let channel_idx = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    // message to a group
                    let master_key = master_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid master key"))?;
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
                    log::warn!("unhandled message from us");
                    return Ok(());
                };

                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let attachments = self.save_attachments(attachment_pointers).await;
                let message = Message {
                    quote,
                    ..Message::new(user_id, body, timestamp, attachments)
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
                    body,
                    group_v2,
                    timestamp: Some(timestamp),
                    profile_key: Some(profile_key),
                    quote,
                    attachments: attachment_pointers,
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

                    (channel_idx, from)
                } else {
                    // incoming direct message
                    let name = self
                        .ensure_user_is_known(uuid, profile_key, phone_number)
                        .await
                        .to_string();
                    let channel_idx = self.ensure_contact_channel_exists(uuid, &name).await;
                    let from = self.data.channels.items[channel_idx].name.clone();
                    // Reset typing notification as the Tipyng::Stop are not always sent by the server when a message is sent.
                    self.data.channels.items[channel_idx].reset_writing(uuid);

                    (channel_idx, from)
                };

                let attachments = self.save_attachments(attachment_pointers).await;
                self.notify_about_message(&from, body.as_deref(), &attachments);

                // Send "Delivered" receipt
                self.add_receipt_event(ReceiptEvent::new(uuid, timestamp, Receipt::Received));

                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let message = Message {
                    quote,
                    ..Message::new(uuid, body, timestamp, attachments)
                };

                if message.is_empty() {
                    return Ok(());
                }

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
                    read,
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
                read.into_iter().for_each(|r| {
                    self.handle_receipt(
                        Uuid::from_str(r.sender_uuid.unwrap().as_str()).unwrap(),
                        1,
                        vec![r.timestamp.unwrap()],
                    )
                });
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
                ContentBody::ReceiptMessage(ReceiptMessage {
                    r#type: Some(typ),
                    timestamp: timestamps,
                }),
            ) => {
                self.handle_receipt(sender_uuid, typ, timestamps);
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
                ContentBody::TypingMessage(TypingMessage {
                    timestamp: Some(timest),
                    group_id,
                    action: Some(act),
                }),
            ) => {
                let _ =
                    self.handle_typing(sender_uuid, group_id, TypingAction::from_i32(act), timest);
                return Ok(());
            }

            _ => return Ok(()),
        };

        self.add_message_to_channel(channel_idx, message);

        Ok(())
    }

    fn notify_about_message(&mut self, from: &str, body: Option<&str>, attachments: &[Attachment]) {
        let attachments_text = notification_text_for_attachments(attachments);
        let notification = [body, attachments_text.as_deref()]
            .into_iter()
            .flatten()
            .join(" ");
        if !notification.is_empty() {
            self.notify(from, &notification);
        }
    }

    pub fn step_receipts(&mut self) -> anyhow::Result<()> {
        if self.receipt_handler.step(self.signal_manager.as_ref()) {
            // No need to save if no receipt was sent
            self.save()
        } else {
            Ok(())
        }
    }

    fn handle_typing(
        &mut self,
        sender_uuid: Uuid,
        group_id: Option<Vec<u8>>,
        action: TypingAction,
        _timestamp: u64,
    ) -> Result<(), ()> {
        if let Some(gid) = group_id {
            // It's in a group
            let group = self
                .data
                .channels
                .items
                .iter_mut()
                .find(|c| {
                    if let ChannelId::Group(gid_other) = c.id {
                        gid_other[..] == gid[..]
                    } else {
                        false
                    }
                })
                .unwrap();
            if let TypingSet::GroupTyping(ref mut hash_set) = group.typing {
                match action {
                    TypingAction::Started => {
                        hash_set.insert(sender_uuid);
                    }
                    TypingAction::Stopped => {
                        hash_set.remove(&sender_uuid);
                    }
                }
            } else {
                log::error!("Got a single typing hash set on a group.");
            }
        } else {
            let chan = self
                .data
                .channels
                .items
                .iter_mut()
                .find(|c| {
                    if let ChannelId::User(other_uuid) = c.id {
                        if other_uuid == sender_uuid {
                            return true;
                        }
                    }
                    false
                })
                .unwrap();

            if let TypingSet::SingleTyping(_) = chan.typing {
                match action {
                    TypingAction::Started => {
                        chan.typing = TypingSet::SingleTyping(true);
                    }
                    TypingAction::Stopped => {
                        chan.typing = TypingSet::SingleTyping(false);
                    }
                }
            } else {
                log::error!("Got a single typing hash set on a group.");
            }
        }
        Ok(())
    }

    pub fn add_receipt_event(&mut self, event: ReceiptEvent) {
        self.receipt_handler.add_receipt_event(event);
    }

    fn handle_receipt(&mut self, sender_uuid: Uuid, typ: i32, timestamps: Vec<u64>) {
        let earliest = timestamps.iter().min().unwrap();
        for c in self.data.channels.items.iter_mut() {
            match c.id {
                ChannelId::User(other_uuid) if other_uuid == sender_uuid => {
                    c.messages.items.iter_mut().rev().fold_while(0, |_, b| {
                        match b.arrived_at.cmp(earliest) {
                            std::cmp::Ordering::Less => Done(0),
                            _ => {
                                if timestamps.contains(&b.arrived_at) {
                                    b.receipt = b.receipt.update(Receipt::from_i32(typ));
                                }
                                Continue(0)
                            }
                        }
                    });
                }
                ChannelId::Group(_) => {
                    if let Some(ref g_data) = c.group_data {
                        if g_data.members.contains(&sender_uuid) {
                            c.messages.items.iter_mut().rev().fold_while(0, |_, b| {
                                match b.arrived_at.cmp(earliest) {
                                    std::cmp::Ordering::Less => Done(0),
                                    _ => {
                                        if timestamps.contains(&b.arrived_at) {
                                            b.receipt = b.receipt.update(Receipt::from_i32(typ));
                                        }
                                        Continue(0)
                                    }
                                }
                            });
                        }
                    }
                }
                _ => (),
            }
        }
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
                typing: TypingSet::GroupTyping(HashSet::new()),
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
                typing: TypingSet::SingleTyping(false),
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
                typing: TypingSet::SingleTyping(false),
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
            log::error!("failed to send notification: {}", e);
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

    async fn save_attachments(
        &mut self,
        attachment_pointers: Vec<AttachmentPointer>,
    ) -> Vec<Attachment> {
        let mut attachments = vec![];
        for attachment_pointer in attachment_pointers {
            match self
                .signal_manager
                .save_attachment(attachment_pointer)
                .await
            {
                Ok(attachment) => attachments.push(attachment),
                Err(e) => log::warn!("failed to save attachment: {}", e),
            }
        }
        attachments
    }

    pub fn toggle_help(&mut self) {
        self.display_help = !self.display_help;
    }

    pub fn toggle_search(&mut self) {
        self.is_searching = !self.is_searching;
    }

    pub fn is_help(&self) -> bool {
        self.display_help
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
        log::error!("failed to open {}: {}", url, e);
    }
    Some(())
}

fn notification_text_for_attachments(attachments: &[Attachment]) -> Option<String> {
    match attachments.len() {
        0 => None,
        1 => Some("<attachment>".into()),
        n => Some(format!("<attachments ({n})>")),
    }
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
                receipt: Default::default(),
            }]),
            unread_messages: 1,
            typing: TypingSet::GroupTyping(HashSet::new()),
        });
        app.data.channels.state.select(Some(0));

        (app, sent_messages)
    }

    #[test]
    fn test_send_input() {
        let (mut app, sent_messages) = test_app();
        let input = "Hello, World!";
        for c in input.chars() {
            app.get_input().put_char(c);
        }
        app.send_input(0).unwrap();

        let sent = sent_messages.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].message.as_ref().unwrap(), input);

        assert_eq!(app.data.channels.items[0].unread_messages, 0);

        assert_eq!(app.get_input().data, "");
    }

    #[test]
    fn test_send_input_with_emoji() {
        let (mut app, sent_messages) = test_app();
        let input = "👻";
        for c in input.chars() {
            app.get_input().put_char(c);
        }

        app.send_input(0).unwrap();

        let sent = sent_messages.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].message.as_ref().unwrap(), input);

        assert_eq!(app.get_input().data, "");
    }

    #[test]
    fn test_send_input_with_emoji_codepoint() {
        let (mut app, sent_messages) = test_app();
        let input = ":thumbsup:";
        for c in input.chars() {
            app.get_input().put_char(c);
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

        app.get_input().put_char('👍');
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
            app.get_input().put_char(c);
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
            app.get_input().put_char(c);
        }
        app.add_reaction(0);

        assert_eq!(app.get_input().data, ":thumbsup");
        let reactions = &app.data.channels.items[0].messages.items[0].reactions;
        assert!(reactions.is_empty());
    }
}
