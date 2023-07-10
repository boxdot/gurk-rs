use crate::channels::SelectChannel;
use crate::config::Config;
use crate::data::{BodyRange, Channel, ChannelId, Message, TypingAction, TypingSet};
use crate::event::Event;
use crate::input::Input;
use crate::receipt::{Receipt, ReceiptEvent, ReceiptHandler};
use crate::signal::{
    Attachment, GroupIdentifierBytes, GroupMasterKeyBytes, ProfileKeyBytes, ResolvedGroup,
    SignalManager,
};
use crate::storage::{MessageId, Storage};
use crate::util::{self, LazyRegex, StatefulList, ATTACHMENT_REGEX, URL_REGEX};
use std::io::Cursor;

use anyhow::{anyhow, bail, Context as _};
use arboard::Clipboard;
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use image::codecs::png::PngEncoder;
use image::{ImageBuffer, ImageEncoder, Rgba};
use itertools::Itertools;
use notify_rust::Notification;
use phonenumber::Mode;
use presage::prelude::proto::{AttachmentPointer, ReceiptMessage, TypingMessage};
use presage::prelude::{
    content::{ContentBody, DataMessage, Metadata, SyncMessage},
    proto::{
        data_message::{Reaction, Sticker},
        sync_message::Sent,
        GroupContextV2,
    },
    AttachmentSpec, Content, ServiceAddress,
};
use regex_automata::Regex;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::future::Future;
use std::path::Path;
use std::time::Duration;

/// Amount of time to skip contacts sync after the last sync
const CONTACTS_SYNC_DEADLINE_SEC: i64 = 60 * 60 * 24; // 1 day
const CONTACTS_SYNC_TIMEOUT: Duration = Duration::from_secs(20);

pub struct App {
    pub config: Config,
    signal_manager: Box<dyn SignalManager>,
    pub storage: Box<dyn Storage>,
    pub channels: StatefulList<ChannelId>,
    pub messages: BTreeMap<ChannelId, StatefulList<u64 /* arrived at*/>>,
    pub user_id: Uuid,
    pub should_quit: bool,
    url_regex: LazyRegex,
    attachment_regex: LazyRegex,
    display_help: bool,
    receipt_handler: ReceiptHandler,
    pub input: Input,
    pub is_multiline_input: bool,
    pub(crate) select_channel: SelectChannel,
    clipboard: Option<Clipboard>,
    event_tx: mpsc::UnboundedSender<Event>,
}

impl App {
    pub fn try_new(
        config: Config,
        signal_manager: Box<dyn SignalManager>,
        storage: Box<dyn Storage>,
    ) -> anyhow::Result<(Self, mpsc::UnboundedReceiver<Event>)> {
        let user_id = signal_manager.user_id();

        // build index of channels and messages for using them as lists content
        let mut channels: StatefulList<ChannelId> = Default::default();
        let mut messages: BTreeMap<_, StatefulList<_>> = BTreeMap::new();
        for channel in storage.channels() {
            channels.items.push(channel.id);
            let channel_messages = &mut messages.entry(channel.id).or_default().items;
            for message in storage.messages(channel.id) {
                channel_messages.push(message.arrived_at);
            }
        }
        channels.items.sort_unstable_by_key(|channel_id| {
            let last_message_arrived_at = storage
                .messages(*channel_id)
                .rev()
                .next()
                .map(|msg| msg.arrived_at);
            let channel_name = storage
                .channel(*channel_id)
                .map(|channel| channel.name.clone());
            (Reverse(last_message_arrived_at), channel_name)
        });
        channels.next();

        let clipboard = Clipboard::new()
            .map_err(|error| warn!(%error, "clipboard disabled"))
            .ok();

        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let app = Self {
            config,
            signal_manager,
            user_id,
            storage,
            channels,
            messages,
            should_quit: false,
            url_regex: LazyRegex::new(URL_REGEX),
            attachment_regex: LazyRegex::new(ATTACHMENT_REGEX),
            display_help: false,
            receipt_handler: ReceiptHandler::new(),
            input: Default::default(),
            is_multiline_input: false,
            select_channel: Default::default(),
            clipboard,
            event_tx,
        };
        Ok((app, event_rx))
    }

    pub fn get_input(&mut self) -> &mut Input {
        if self.select_channel.is_shown {
            &mut self.select_channel.input
        } else {
            &mut self.input
        }
    }

    pub fn writing_people(&self, channel: &Channel) -> Option<String> {
        if channel.is_writing() {
            let uuids: Box<dyn Iterator<Item = Uuid>> = match &channel.typing {
                TypingSet::GroupTyping(uuids) => Box::new(uuids.iter().copied()),
                TypingSet::SingleTyping(a) => {
                    if *a {
                        Box::new(std::iter::once(channel.user_id().unwrap()))
                    } else {
                        Box::new(std::iter::empty())
                    }
                }
            };
            Some(format!(
                "[{}] writing...",
                uuids.map(|id| self.name_by_id(id)).format(", ")
            ))
        } else {
            None
        }
    }

    // Resolves name of a user by their id
    //
    // The resolution is done in the following way:
    //
    // 1. It's us => name from config
    // 2. User id is in presage's signal manager (that is, it is a known contact from our address
    //    book) => use it,
    // 3. User id is in the gurk's user name table (custom name) => use it,
    // 4. give up with "Unknown User"
    pub fn name_by_id(&self, id: Uuid) -> String {
        if self.user_id == id {
            // it's me
            self.config.user.name.clone()
        } else if let Some(contact) = self
            .signal_manager
            .contact_by_id(id)
            .ok()
            .flatten()
            .filter(|contact| !contact.name.is_empty())
        {
            // user is known via our contact list
            contact.name
        } else if let Some(name) = self.storage.name(id) {
            // user should be at least known via their profile or phone number
            name.into_owned()
        } else {
            // give up
            "Unknown User".to_string()
        }
    }

    pub fn channel_name<'a>(&self, channel: &'a Channel) -> Cow<'a, str> {
        if let Some(id) = channel.user_id() {
            self.name_by_id(id).into()
        } else {
            (&channel.name).into()
        }
    }

    pub async fn on_key(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        match key.code {
            KeyCode::Char('\r') => self.get_input().put_char('\n'),
            KeyCode::Enter => {
                if !self.select_channel.is_shown {
                    if key.modifiers.contains(KeyModifiers::ALT) {
                        self.is_multiline_input = !self.is_multiline_input;
                    } else if self.is_multiline_input {
                        self.get_input().new_line();
                    } else if !self.input.data.is_empty() {
                        if let Some(idx) = self.channels.state.selected() {
                            self.send_input(idx);
                        }
                    } else {
                        // input is empty
                        self.try_open_url();
                    }
                } else if self.select_channel.is_shown {
                    if let Some(channel_id) = self.select_channel.selected_channel_id().copied() {
                        self.select_channel.is_shown = false;
                        let (idx, _) = self
                            .channels
                            .items
                            .iter()
                            .enumerate()
                            .find(|(_, &id)| id == channel_id)
                            .context("channel disappeared during channel select popup")?;
                        self.channels.state.select(Some(idx));
                    }
                }
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
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if !self.select_channel.is_shown {
                    self.select_channel.reset(&*self.storage);
                }
                self.select_channel.is_shown = !self.select_channel.is_shown;
            }
            KeyCode::Esc => {
                if self.select_channel.is_shown {
                    self.select_channel.is_shown = false;
                } else {
                    self.reset_message_selection();
                }
            }
            KeyCode::Char(c) => self.get_input().put_char(c),
            KeyCode::Tab => {
                if let Some(idx) = self.channels.state.selected() {
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
        // Note: to make the borrow checker happy, we have to use distinct fields here, and no
        // methods that borrow self mutably.
        let channel_id = self.channels.selected_item()?;
        let messages = self.messages.get(channel_id)?;
        let idx = messages.state.selected()?;
        let idx = messages.items.len().checked_sub(idx + 1)?;
        let arrived_at = messages.items.get(idx)?;
        let message = self
            .storage
            .message(MessageId::new(*channel_id, *arrived_at))?;
        let re = self.url_regex.compiled();
        open_url(&message, re)?;
        self.reset_message_selection();
        Some(())
    }

    fn selected_message(&self) -> Option<Cow<Message>> {
        // Messages are shown in reversed order => selected is reversed
        let channel_id = self.channels.selected_item()?;
        let messages = self.messages.get(channel_id)?;
        let idx = messages.state.selected()?;
        let idx = messages.items.len().checked_sub(idx + 1)?;
        let arrived_at = messages.items.get(idx)?;
        let message_id = MessageId::new(*channel_id, *arrived_at);
        self.storage.message(message_id)
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
        let channel = self.storage.channel(self.channels.items[channel_idx])?;
        let message = self.selected_message()?;
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
            .send_reaction(&channel, &message, emoji.clone(), remove);

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

        Some(())
    }

    fn reset_message_selection(&mut self) {
        if let Some(channel_id) = self.channels.selected_item() {
            if let Some(messages) = self.messages.get_mut(channel_id) {
                messages.state.select(None);
                messages.rendered = Default::default();
            }
        }
    }

    fn take_input(&mut self) -> String {
        self.get_input().take()
    }

    fn send_input(&mut self, channel_idx: usize) {
        let input = self.take_input();
        let (input, attachments) = self.extract_attachments(&input);
        let channel_id = self.channels.items[channel_idx];
        let channel = self
            .storage
            .channel(channel_id)
            .expect("non-existent channel");
        let quote = self.selected_message();
        let (sent_message, response) =
            self.signal_manager
                .send_text(&channel, input, quote.as_deref(), attachments);

        let message_id = MessageId::new(channel_id, sent_message.arrived_at);
        let tx = self.event_tx.clone();
        tokio::spawn(async move {
            if let Ok(result) = response.await {
                tx.send(Event::SentTextResult { message_id, result })
                    .expect("event sender gone");
            } else {
                error!(?message_id, "response for sending message was lost");
            }
        });

        let sent_message = self.storage.store_message(channel_id, sent_message);
        self.messages
            .get_mut(&channel_id)
            .expect("non-existent channel")
            .items
            .push(sent_message.arrived_at);

        let sent_with_quote = sent_message.quote.is_some();
        self.reset_unread_messages();
        if sent_with_quote {
            self.reset_message_selection();
        }
        self.bubble_up_channel(channel_idx);
    }

    pub fn select_previous_channel(&mut self) {
        self.reset_unread_messages();
        self.channels.previous();
    }

    pub fn select_next_channel(&mut self) {
        self.reset_unread_messages();
        self.channels.next();
    }

    pub fn on_pgup(&mut self) {
        if let Some(channel_id) = self.channels.selected_item() {
            self.messages
                .get_mut(channel_id)
                .expect("non-existent channel")
                .next();
        }
    }

    pub fn on_pgdn(&mut self) {
        if let Some(channel_id) = self.channels.selected_item() {
            self.messages
                .get_mut(channel_id)
                .expect("non-existent channel")
                .previous()
        }
    }

    pub fn reset_unread_messages(&mut self) {
        if let Some(channel_id) = self.channels.selected_item() {
            if let Some(channel) = self.storage.channel(*channel_id) {
                if channel.unread_messages > 0 {
                    let mut channel = channel.into_owned();
                    channel.unread_messages = 0;
                    self.storage.store_channel(channel);
                }
            }
        }
    }

    pub async fn on_message(&mut self, content: Content) -> anyhow::Result<()> {
        // tracing::debug!("incoming: {:#?}", content);

        #[cfg(feature = "dev")]
        if self.config.developer.dump_raw_messages {
            if let Err(e) = crate::dev::dump_raw_message(&content) {
                warn!(error = %e, "failed to dump raw message");
            }
        }

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
                                    mut body,
                                    attachments: attachment_pointers,
                                    sticker,
                                    body_ranges,
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if destination_uuid.parse() == Ok(user_id) => {
                let channel_idx = self.ensure_own_channel_exists();
                let attachments = self.save_attachments(attachment_pointers).await;
                add_emoji_from_sticker(&mut body, sticker);

                let body_ranges = body_ranges.into_iter().filter_map(BodyRange::from_proto);

                let message = Message::new(user_id, body, body_ranges, timestamp, attachments);
                (channel_idx, message)
            }
            // Direct/group message by us from a different device
            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: sender_uuid, ..
                        },
                    ..
                },
                ContentBody::SynchronizeMessage(SyncMessage {
                    sent:
                        Some(Sent {
                            destination_uuid,
                            timestamp: Some(timestamp),
                            message:
                                Some(DataMessage {
                                    mut body,
                                    profile_key: Some(profile_key),
                                    group_v2,
                                    quote,
                                    attachments: attachment_pointers,
                                    sticker,
                                    body_ranges,
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
                } else if let Some(destination_uuid) = destination_uuid {
                    let profile_key = profile_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid profile key"))?;
                    let destination_uuid = Uuid::parse_str(&destination_uuid).unwrap();
                    let name = self.name_by_id(destination_uuid);
                    self.ensure_user_is_known(destination_uuid, profile_key)
                        .await;
                    self.ensure_contact_channel_exists(destination_uuid, &name)
                        .await
                } else {
                    bail!("message without a group context and without a destination uuid");
                };

                add_emoji_from_sticker(&mut body, sticker);
                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let attachments = self.save_attachments(attachment_pointers).await;
                let body_ranges = body_ranges.into_iter().filter_map(BodyRange::from_proto);
                let message = Message {
                    quote,
                    ..Message::new(user_id, body, body_ranges, timestamp, attachments)
                };

                (channel_idx, message)
            }
            // Incoming direct/group message
            (
                Metadata {
                    sender: ServiceAddress { uuid, .. },
                    ..
                },
                ContentBody::DataMessage(DataMessage {
                    mut body,
                    group_v2,
                    timestamp: Some(timestamp),
                    profile_key: Some(profile_key),
                    quote,
                    attachments: attachment_pointers,
                    sticker,
                    body_ranges,
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
                    let profile_key = profile_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid profile key"))?;
                    let master_key = master_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid group master key"))?;
                    let channel_idx = self
                        .ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?;

                    self.ensure_user_is_known(uuid, profile_key).await;
                    let from = self.name_by_id(uuid);

                    (channel_idx, from)
                } else {
                    // incoming direct message
                    let profile_key = profile_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid profile key"))?;
                    self.ensure_user_is_known(uuid, profile_key).await;
                    let name = self.name_by_id(uuid);
                    let channel_idx = self.ensure_contact_channel_exists(uuid, &name).await;
                    // Reset typing notification as the Tipyng::Stop are not always sent by the server when a message is sent.
                    let channel_id = self.channels.items[channel_idx];
                    let mut channel = self
                        .storage
                        .channel(channel_id)
                        .expect("non-existent channel")
                        .into_owned();
                    let from = channel.name.clone();
                    if channel.reset_writing(uuid) {
                        self.storage.store_channel(channel);
                    }
                    (channel_idx, from)
                };

                add_emoji_from_sticker(&mut body, sticker);

                let attachments = self.save_attachments(attachment_pointers).await;
                self.notify_about_message(&from, body.as_deref(), &attachments);

                // Send "Delivered" receipt
                self.add_receipt_event(ReceiptEvent::new(uuid, timestamp, Receipt::Delivered));

                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let body_ranges = body_ranges.into_iter().filter_map(BodyRange::from_proto);
                let message = Message {
                    quote,
                    ..Message::new(uuid, body, body_ranges, timestamp, attachments)
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
                            uuid: sender_uuid, ..
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
                        Uuid::parse_str(r.sender_uuid.unwrap().as_str()).unwrap(),
                        Receipt::Read,
                        vec![r.timestamp.unwrap()],
                    );
                });
                return Ok(());
            }
            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: sender_uuid, ..
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
                            uuid: sender_uuid, ..
                        },
                    ..
                },
                ContentBody::ReceiptMessage(ReceiptMessage {
                    r#type: Some(receipt_type),
                    timestamp: timestamps,
                }),
            ) => {
                let receipt = Receipt::from_i32(receipt_type);
                self.handle_receipt(sender_uuid, receipt, timestamps);
                return Ok(());
            }

            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: sender_uuid, ..
                        },
                    ..
                },
                ContentBody::TypingMessage(TypingMessage {
                    timestamp: Some(timest),
                    group_id,
                    action: Some(act),
                }),
            ) => {
                let group_id_bytes = match group_id.map(TryInto::try_into).transpose() {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        error!("invalid group id: failed to convert to group identified bytes");
                        return Ok(());
                    }
                };
                if self
                    .handle_typing(
                        sender_uuid,
                        group_id_bytes,
                        TypingAction::from_i32(act),
                        timest,
                    )
                    .is_err()
                {
                    error!("failed to handle typing: unknown error");
                }
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

    pub fn step_receipts(&mut self) {
        self.receipt_handler.step(self.signal_manager.as_ref());
    }

    fn handle_typing(
        &mut self,
        sender_uuid: Uuid,
        group_id: Option<GroupIdentifierBytes>,
        action: TypingAction,
        _timestamp: u64,
    ) -> Result<(), ()> {
        if let Some(gid) = group_id {
            let mut channel = self
                .storage
                .channel(ChannelId::Group(gid))
                .ok_or(())?
                .into_owned();
            if let TypingSet::GroupTyping(ref mut hash_set) = channel.typing {
                match action {
                    TypingAction::Started => {
                        hash_set.insert(sender_uuid);
                    }
                    TypingAction::Stopped => {
                        hash_set.remove(&sender_uuid);
                    }
                }
                self.storage.store_channel(channel);
            } else {
                error!("Got a single typing instead of hash set on a group");
            }
        } else {
            let mut channel = self
                .storage
                .channel(ChannelId::User(sender_uuid))
                .ok_or(())?
                .into_owned();
            if let TypingSet::SingleTyping(_) = channel.typing {
                match action {
                    TypingAction::Started => {
                        channel.typing = TypingSet::SingleTyping(true);
                    }
                    TypingAction::Stopped => {
                        channel.typing = TypingSet::SingleTyping(false);
                    }
                }
                self.storage.store_channel(channel);
            } else {
                error!("Got a hash set instead of single typing on a direct chat");
            }
        }
        Ok(())
    }

    pub fn add_receipt_event(&mut self, event: ReceiptEvent) {
        self.receipt_handler.add_receipt_event(event);
    }

    fn handle_receipt(&mut self, sender_uuid: Uuid, receipt: Receipt, mut timestamps: Vec<u64>) {
        let sender_channels: Vec<ChannelId> = self
            .storage
            .channels()
            .filter(|channel| match channel.id {
                ChannelId::User(uuid) => uuid == sender_uuid,
                ChannelId::Group(_) => channel
                    .group_data
                    .as_ref()
                    .map(|group_data| group_data.members.contains(&sender_uuid))
                    .unwrap_or(false),
            })
            .map(|channel| channel.id)
            .collect();

        timestamps.sort_unstable_by_key(|&ts| Reverse(ts));
        if timestamps.is_empty() {
            return;
        }

        let mut found_channel_id = None;
        let mut messages_to_store = Vec::new();

        'outer: for channel_id in sender_channels {
            let mut messages = self.storage.messages(channel_id).rev();
            for &ts in &timestamps {
                // Note: `&mut` is needed to advance the iterator `messages` with each `ts`.
                // Since these are sorted in reverse order, we can continue advancing messages
                // without consuming them.
                if let Some(msg) = (&mut messages)
                    .take_while(|msg| msg.arrived_at >= ts)
                    .find(|msg| msg.arrived_at == ts)
                {
                    let mut msg = msg.into_owned();
                    if msg.receipt < receipt {
                        msg.receipt = msg.receipt.max(receipt);
                        messages_to_store.push(msg);
                    }
                    found_channel_id = Some(channel_id);
                }
            }

            if found_channel_id.is_some() {
                // if one ts was found, then all other ts have to be in the same channel
                break 'outer;
            }
        }

        if let Some(channel_id) = found_channel_id {
            for message in messages_to_store {
                self.storage.store_message(channel_id, message);
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
        let mut message = self
            .storage
            .message(MessageId::new(channel_id, target_sent_timestamp))?
            .into_owned();

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
        let message = self.storage.store_message(channel_id, message);

        if is_added && channel_id != ChannelId::User(self.user_id) {
            // Notification
            let mut notification = format!("reacted {emoji}");
            if let Some(text) = message.message.as_ref() {
                notification.push_str(" to: ");
                notification.push_str(text);
            }

            // makes borrow checker happy
            let channel = self.storage.channel(channel_id)?;
            let channel_name = channel.name.clone();

            let sender_name = self.name_by_id(sender_uuid);
            let summary = if let ChannelId::Group(_) = channel_id {
                Cow::from(format!("{sender_name} in {channel_name}"))
            } else {
                Cow::from(sender_name)
            };

            if notify {
                self.notify(&summary, &format!("{summary} {notification}"));
            }

            let channel_idx = self
                .channels
                .items
                .iter()
                .position(|id| id == &channel_id)
                .expect("non-existent channel");
            self.touch_channel(channel_idx);
        }

        Some(())
    }

    async fn ensure_group_channel_exists(
        &mut self,
        master_key: GroupMasterKeyBytes,
        revision: u32,
    ) -> anyhow::Result<usize> {
        let channel_id = ChannelId::from_master_key_bytes(master_key)?;
        if let Some(channel_idx) = self.channels.items.iter().position(|id| id == &channel_id) {
            // existing channel
            let channel = self
                .storage
                .channel(channel_id)
                .expect("non-existent channel");

            let is_stale = match channel.group_data.as_ref() {
                Some(group_data) => group_data.revision != revision,
                None => true,
            };
            if is_stale {
                let ResolvedGroup {
                    name,
                    group_data,
                    profile_keys,
                } = self.signal_manager.resolve_group(master_key).await?;

                let mut channel = channel.into_owned();

                self.ensure_users_are_known(
                    group_data
                        .members
                        .iter()
                        .copied()
                        .zip(profile_keys.into_iter()),
                )
                .await;

                channel.name = name;
                channel.group_data = Some(group_data);
                self.storage.store_channel(channel);
            }
            Ok(channel_idx)
        } else {
            // new channel
            let ResolvedGroup {
                name,
                group_data,
                profile_keys,
            } = self.signal_manager.resolve_group(master_key).await?;

            self.ensure_users_are_known(
                group_data
                    .members
                    .iter()
                    .copied()
                    .zip(profile_keys.into_iter()),
            )
            .await;

            let channel = Channel {
                id: channel_id,
                name,
                group_data: Some(group_data),
                unread_messages: 0,
                typing: TypingSet::GroupTyping(Default::default()),
            };
            self.storage.store_channel(channel);

            let channel_idx = self.channels.items.len();
            self.channels.items.push(channel_id);

            Ok(channel_idx)
        }
    }

    async fn ensure_user_is_known(&mut self, uuid: Uuid, profile_key: ProfileKeyBytes) {
        // is_known <=>
        //   * in names, or
        //   * is not a phone numbers, or
        //   * is not their uuid
        let is_known = self
            .storage
            .name(uuid)
            .filter(|name| !util::is_phone_number(name) && Uuid::parse_str(name) != Ok(uuid))
            .is_some();
        if !is_known {
            if let Some(name) = self
                .signal_manager
                .contact_by_id(uuid)
                .ok()
                .flatten()
                .and_then(|c| {
                    c.phone_number
                        .map(|p| p.format().mode(Mode::E164).to_string())
                })
            {
                // resolved from contact list
                self.storage.store_name(uuid, name);
            } else if let Some(name) = self
                .signal_manager
                .resolve_name_from_profile(uuid, profile_key)
                .await
            {
                // resolved from signal service via their profile
                self.storage.store_name(uuid, name);
            } else {
                // failed to resolve
                self.storage.store_name(uuid, uuid.to_string());
            }
        }
    }

    async fn ensure_users_are_known(
        &mut self,
        users_with_keys: impl Iterator<Item = (Uuid, ProfileKeyBytes)>,
    ) {
        // TODO: Run in parallel
        for (uuid, profile_key) in users_with_keys {
            self.ensure_user_is_known(uuid, profile_key).await;
        }
    }

    fn ensure_own_channel_exists(&mut self) -> usize {
        let user_id = self.user_id;
        if let Some(channel_idx) = self
            .channels
            .items
            .iter()
            .position(|channel_id| channel_id == &user_id)
        {
            channel_idx
        } else {
            let channel = Channel {
                id: user_id.into(),
                name: self.config.user.name.clone(),
                group_data: None,
                unread_messages: 0,
                typing: TypingSet::SingleTyping(false),
            };
            let channel = self.storage.store_channel(channel);

            let channel_idx = self.channels.items.len();
            self.channels.items.push(channel.id);

            channel_idx
        }
    }

    async fn ensure_contact_channel_exists(&mut self, uuid: Uuid, name: &str) -> usize {
        if let Some(channel_idx) = self
            .channels
            .items
            .iter()
            .position(|channel_id| channel_id == &uuid)
        {
            let channel = self
                .storage
                .channel(uuid.into())
                .expect("non-existent channel");
            if channel.name != name {
                let mut channel = channel.into_owned();
                channel.name = name.to_string();
                self.storage.store_channel(channel);
            }
            channel_idx
        } else {
            let channel = Channel {
                id: uuid.into(),
                name: name.to_string(),
                group_data: None,
                unread_messages: 0,
                typing: TypingSet::SingleTyping(false),
            };
            let channel = self.storage.store_channel(channel);

            let channel_idx = self.channels.items.len();
            self.channels.items.push(channel.id);

            channel_idx
        }
    }

    fn add_message_to_channel(&mut self, channel_idx: usize, message: Message) {
        let channel_id = self.channels.items[channel_idx];

        let message = self.storage.store_message(channel_id, message);

        let messages = self.messages.entry(channel_id).or_default();
        messages.items.push(message.arrived_at);

        if let Some(idx) = messages.state.selected() {
            // keep selection on the old message
            messages.state.select(Some(idx + 1));
        }

        self.touch_channel(channel_idx);
    }

    fn touch_channel(&mut self, channel_idx: usize) {
        if self.channels.state.selected() != Some(channel_idx) {
            let channel_id = self.channels.items[channel_idx];
            let mut channel = self
                .storage
                .channel(channel_id)
                .expect("non-existent channel")
                .into_owned();
            channel.unread_messages += 1;
            self.storage.store_channel(channel);
        } else {
            self.reset_unread_messages();
        }

        self.bubble_up_channel(channel_idx);
    }

    fn bubble_up_channel(&mut self, channel_idx: usize) {
        // bubble up channel to the beginning of the list
        let channels = &mut self.channels;
        for (prev, next) in (0..channel_idx).zip(1..channel_idx + 1).rev() {
            channels.items.swap(prev, next);
        }
        match channels.state.selected() {
            Some(selected_idx) if selected_idx == channel_idx => channels.state.select(Some(0)),
            Some(selected_idx) if selected_idx < channel_idx => {
                channels.state.select(Some(selected_idx + 1));
            }
            _ => {}
        }
    }

    fn notify(&self, summary: &str, text: &str) {
        if self.config.notifications {
            if let Err(e) = Notification::new().summary(summary).body(text).show() {
                error!("failed to send notification: {}", e);
            }
        }
    }

    fn extract_attachments(&mut self, input: &str) -> (String, Vec<(AttachmentSpec, Vec<u8>)>) {
        let mut offset = 0;
        let mut clean_input = String::new();

        let re = self.attachment_regex.compiled();
        let attachments = re.find_iter(input.as_bytes()).filter_map(|(start, end)| {
            let path_str = &input[start..end].strip_prefix("file://")?;

            let (contents, content_type, file_name) = if path_str.starts_with(&"clip") {
                let img = self.clipboard.as_mut()?.get_image().ok()?;

                let png: ImageBuffer<Rgba<_>, _> =
                    ImageBuffer::from_raw(img.width as _, img.height as _, img.bytes)?;

                let mut bytes = Vec::new();
                let mut cursor = Cursor::new(&mut bytes);
                let encoder = PngEncoder::new(&mut cursor);

                let data: Vec<_> = png.into_raw().into_iter().map(|b| b.swap_bytes()).collect();
                encoder
                    .write_image(
                        &data,
                        img.width as _,
                        img.height as _,
                        image::ColorType::Rgba8,
                    )
                    .ok()?;

                (
                    bytes,
                    "image/png".to_string(),
                    Some("clipboard.png".to_string()),
                )
            } else {
                let path = Path::new(path_str);
                let contents = std::fs::read(path).ok()?;
                let content_type = mime_guess::from_path(path)
                    .first()
                    .map(|mime| mime.essence_str().to_string())
                    .unwrap_or_default();
                let file_name = path.file_name().map(|f| f.to_string_lossy().into());

                (contents, content_type, file_name)
            };

            clean_input.push_str(input[offset..start].trim_end_matches(""));
            offset = end;

            let spec = AttachmentSpec {
                content_type,
                length: contents.len(),
                file_name,
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
                Err(e) => warn!("failed to save attachment: {}", e),
            }
        }
        attachments
    }

    pub fn toggle_help(&mut self) {
        self.display_help = !self.display_help;
    }

    pub fn is_help(&self) -> bool {
        self.display_help
    }

    pub fn request_contacts_sync(
        &self,
    ) -> Option<impl Future<Output = anyhow::Result<DateTime<Utc>>> + 'static> {
        let now = Utc::now();
        let metadata = self.storage.metadata();
        let do_sync = metadata
            .contacts_sync_request_at
            .map(|dt| dt + chrono::Duration::seconds(CONTACTS_SYNC_DEADLINE_SEC) < now)
            .unwrap_or(true);
        let signal_manager = self.signal_manager.clone_boxed();
        do_sync.then_some(async move {
            info!(timeout =? CONTACTS_SYNC_TIMEOUT, "requesting contact sync");
            tokio::time::timeout(
                CONTACTS_SYNC_TIMEOUT,
                signal_manager.request_contacts_sync(),
            )
            .await??;
            Ok(Utc::now())
        })
    }

    pub fn is_select_channel_shown(&self) -> bool {
        self.select_channel.is_shown
    }

    pub fn select_channel_prev(&mut self) {
        self.select_channel.prev();
    }

    pub fn select_channel_next(&mut self) {
        self.select_channel.next();
    }

    pub fn copy_selection(&mut self) {
        if let Some(message) = self.selected_message() {
            if let Some(text) = message.message.as_ref() {
                let text = text.clone();
                if let Some(clipboard) = self.clipboard.as_mut() {
                    if let Err(error) = clipboard.set_text(text) {
                        error!(%error, "failed to copy text to clipboard");
                    } else {
                        info!("copied selected text to clipboard");
                    }
                }
            }
        }
    }

    pub fn handle_event(&mut self, event: Event) -> anyhow::Result<()> {
        match event {
            Event::SentTextResult { message_id, result } => {
                if let Err(error) = result {
                    let mut message = self
                        .storage
                        .message(message_id)
                        .context("no message")?
                        .into_owned();
                    message.send_failed = Some(error.to_string());
                    self.storage.store_message(message_id.channel_id, message);
                }
            }
        }
        Ok(())
    }
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

fn notification_text_for_attachments(attachments: &[Attachment]) -> Option<String> {
    match attachments.len() {
        0 => None,
        1 => Some("<attachment>".into()),
        n => Some(format!("<attachments ({n})>")),
    }
}

fn add_emoji_from_sticker(body: &mut Option<String>, sticker: Option<Sticker>) {
    if let Some(Sticker { emoji: Some(e), .. }) = sticker {
        *body = Some(format!("<{e}>"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::User;
    use crate::data::GroupData;
    use crate::signal::test::SignalManagerMock;
    use crate::storage::{ForgetfulStorage, MemCache};

    use std::cell::RefCell;
    use std::rc::Rc;

    fn test_app() -> (
        App,
        mpsc::UnboundedReceiver<Event>,
        Rc<RefCell<Vec<Message>>>,
    ) {
        let signal_manager = SignalManagerMock::new();
        let sent_messages = signal_manager.sent_messages.clone();

        let mut storage = MemCache::new(ForgetfulStorage);

        let channel_id = ChannelId::User(Uuid::new_v4());
        let channel = Channel {
            id: channel_id,
            name: "test".to_string(),
            group_data: Some(GroupData {
                master_key_bytes: GroupMasterKeyBytes::default(),
                members: vec![signal_manager.user_id()],
                revision: 1,
            }),
            unread_messages: 1,
            typing: TypingSet::GroupTyping(Default::default()),
        };
        storage.store_channel(channel);
        storage.store_message(
            channel_id,
            Message {
                from_id: signal_manager.user_id(),
                message: Some("First message".to_string()),
                arrived_at: 0,
                quote: Default::default(),
                attachments: Default::default(),
                reactions: Default::default(),
                receipt: Default::default(),
                body_ranges: Default::default(),
                send_failed: Default::default(),
            },
        );

        let user = User {
            name: "Tyler Durden".to_string(),
            phone_number: "+0000000000".to_string(),
        };
        let (mut app, events) = App::try_new(
            Config::with_user(user),
            Box::new(signal_manager),
            Box::new(storage),
        )
        .unwrap();
        app.channels.state.select(Some(0));

        (app, events, sent_messages)
    }

    #[tokio::test]
    async fn test_send_input() {
        let (mut app, mut events, sent_messages) = test_app();
        let input = "Hello, World!";
        for c in input.chars() {
            app.get_input().put_char(c);
        }
        app.send_input(0);

        assert_eq!(sent_messages.borrow().len(), 1);
        let msg = sent_messages.borrow()[0].clone();
        assert_eq!(msg.message.as_ref().unwrap(), input);

        let channel_id = app.channels.items[0];
        let channel = app.storage.channel(channel_id).unwrap();
        assert_eq!(channel.unread_messages, 0);

        assert_eq!(app.get_input().data, "");

        match events.recv().await.unwrap() {
            Event::SentTextResult { message_id, result } => {
                assert_eq!(message_id.arrived_at, msg.arrived_at);
                assert!(result.is_ok());
            }
        }
    }

    #[tokio::test]
    async fn test_send_input_with_emoji() {
        let (mut app, mut events, sent_messages) = test_app();
        let input = "👻";
        for c in input.chars() {
            app.get_input().put_char(c);
        }

        app.send_input(0);

        assert_eq!(sent_messages.borrow().len(), 1);
        let msg = sent_messages.borrow()[0].clone();
        assert_eq!(msg.message.as_ref().unwrap(), input);

        assert_eq!(app.get_input().data, "");

        match events.recv().await.unwrap() {
            Event::SentTextResult { message_id, result } => {
                assert_eq!(message_id.arrived_at, msg.arrived_at);
                assert!(result.is_ok());
            }
        }
    }

    #[tokio::test]
    async fn test_send_input_with_emoji_codepoint() {
        let (mut app, mut events, sent_messages) = test_app();
        let input = ":thumbsup:";
        for c in input.chars() {
            app.get_input().put_char(c);
        }

        app.send_input(0);

        assert_eq!(sent_messages.borrow().len(), 1);
        let msg = sent_messages.borrow()[0].clone();
        assert_eq!(msg.message.as_ref().unwrap(), "👍");

        match events.recv().await.unwrap() {
            Event::SentTextResult { message_id, result } => {
                assert_eq!(message_id.arrived_at, msg.arrived_at);
                assert!(result.is_ok());
            }
        }
    }

    #[test]
    fn test_add_reaction_with_emoji() {
        let (mut app, _events, _sent_messages) = test_app();

        let channel_id = app.channels.items[0];
        app.messages
            .get_mut(&channel_id)
            .unwrap()
            .state
            .select(Some(0));

        app.get_input().put_char('👍');
        app.add_reaction(0);

        let arrived_at = app.messages[&channel_id].items[0];
        let reactions = &app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0], (app.user_id, "👍".to_string()));
    }

    #[test]
    fn test_add_reaction_with_emoji_codepoint() {
        let (mut app, _events, _sent_messages) = test_app();

        let channel_id = app.channels.items[0];
        app.messages
            .get_mut(&channel_id)
            .unwrap()
            .state
            .select(Some(0));

        for c in ":thumbsup:".chars() {
            app.get_input().put_char(c);
        }
        app.add_reaction(0);

        let arrived_at = app.messages[&channel_id].items[0];
        let reactions = &app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .reactions;
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0], (app.user_id, "👍".to_string()));
    }

    #[test]
    fn test_remove_reaction() {
        let (mut app, _events, _sent_messages) = test_app();

        let channel_id = app.channels.items[0];
        app.messages
            .get_mut(&channel_id)
            .unwrap()
            .state
            .select(Some(0));

        let arrived_at = app.messages[&channel_id].items[0];
        let mut message = app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .into_owned();
        message.reactions.push((app.user_id, "👍".to_string()));
        app.storage.store_message(channel_id, message);
        app.add_reaction(0);

        let reactions = &app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .reactions;
        assert!(reactions.is_empty());
    }

    #[test]
    fn test_add_invalid_reaction() {
        let (mut app, _events, _sent_messages) = test_app();
        let channel_id = app.channels.items[0];
        app.messages
            .get_mut(&channel_id)
            .unwrap()
            .state
            .select(Some(0));

        for c in ":thumbsup".chars() {
            app.get_input().put_char(c);
        }
        app.add_reaction(0);

        assert_eq!(app.get_input().data, ":thumbsup");
        let arrived_at = app.messages[&channel_id].items[0];
        let reactions = &app
            .storage
            .message(MessageId::new(channel_id, arrived_at))
            .unwrap()
            .reactions;
        assert!(reactions.is_empty());
    }
}
