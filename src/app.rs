use crate::config::Config;
use crate::data::{AppData, Channel, ChannelId, ExpireTimer, Message, TypingAction, TypingSet};
use crate::input::Input;
use crate::receipt::{Receipt, ReceiptEvent, ReceiptHandler};
use crate::signal::{Attachment, GroupMasterKeyBytes, ProfileKey, ResolvedGroup, SignalManager};
use crate::storage::Storage;
use crate::util::{self, LazyRegex, StatefulList, ATTACHMENT_REGEX, URL_REGEX};

use anyhow::{anyhow, bail, Context as _};
use chrono::{Duration, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
use tracing::{error, info, warn};
use uuid::Uuid;

use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::convert::TryInto;
use std::path::Path;

/// Amount of time to skip contacts sync after the last sync
const CONTACTS_SYNC_DEADLINE_SEC: i64 = 60 * 60; // 1h

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
    pub input: Input,
    pub search_box: Input,
    pub is_multiline_input: bool,
}

impl App {
    pub fn try_new(
        config: Config,
        signal_manager: Box<dyn SignalManager>,
        storage: Box<dyn Storage>,
    ) -> anyhow::Result<Self> {
        let user_id = signal_manager.user_id();
        let data = storage.load_app_data()?;
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
            input: Default::default(),
            search_box: Default::default(),
            is_multiline_input: false,
        })
    }

    pub fn get_input(&mut self) -> &mut Input {
        if self.is_searching {
            &mut self.search_box
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

    pub fn save(&self) -> anyhow::Result<()> {
        self.storage.save_app_data(&self.data)
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
        } else if let Some(name) = self.data.names.get(&id) {
            // user should be at least known via their profile or phone number
            name.clone()
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

    pub fn on_key(&mut self, key: KeyEvent) -> anyhow::Result<()> {
        match key.code {
            KeyCode::Char('\r') => self.get_input().put_char('\n'),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) && !self.is_searching => {
                self.is_multiline_input = !self.is_multiline_input;
            }
            KeyCode::Enter if self.is_multiline_input && !self.is_searching => {
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

    pub fn get_channel(&mut self, id: usize) -> &mut Channel {
        &mut self.data.channels.items[id]
    }

    pub async fn on_message(&mut self, content: Content) -> anyhow::Result<()> {
        // tracing::info!("incoming: {:#?}", content.body);
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

                let expiration =
                    ExpireTimer::from_delay_s_opt(self.get_channel(channel_idx).expire_timer);

                let message = Message::new(user_id, body, timestamp, attachments, expiration);
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
                                    expire_timer,
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

                let expire_timestamp = ExpireTimer::from_delay_s_opt(expire_timer);

                add_emoji_from_sticker(&mut body, sticker);
                let quote = quote
                    .and_then(|q| Message::from_quote(q, expire_timestamp))
                    .map(Box::new);
                let attachments = self.save_attachments(attachment_pointers).await;
                let message = Message {
                    quote,
                    ..Message::new(user_id, body, timestamp, attachments, expire_timestamp)
                };

                (channel_idx, message)
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
            // Message expiration timer change
            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: Some(uuid), ..
                        },
                    ..
                },
                ContentBody::DataMessage(DataMessage {
                    body: None,
                    group_v2,
                    expire_timer,
                    ..
                }),
            ) => {
                let channel_idx = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    // In a group
                    let master_key = master_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid group master key"))?;

                    self.ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?
                } else {
                    // In a direct message channel
                    let name = self.name_by_id(uuid);

                    self.ensure_contact_channel_exists(uuid, &name).await
                };
                // Save (possibly new) expire timer
                let channel = self.get_channel(channel_idx);
                if channel.expire_timer != expire_timer {
                    tracing::info!(
                        "Expire timer changed in channel {}: {:?}",
                        channel.name,
                        expire_timer
                    );
                    channel.expire_timer = expire_timer;
                }
                return Ok(());
            }
            // Incoming direct/group message
            (
                Metadata {
                    sender:
                        ServiceAddress {
                            uuid: Some(uuid), ..
                        },
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
                    expire_timer,
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
                    let from = self.data.channels.items[channel_idx].name.clone();
                    // Reset typing notification as the Tipyng::Stop are not always sent by the server when a message is sent.
                    self.data.channels.items[channel_idx].reset_writing(uuid);

                    (channel_idx, from)
                };

                add_emoji_from_sticker(&mut body, sticker);

                let attachments = self.save_attachments(attachment_pointers).await;
                self.notify_about_message(&from, body.as_deref(), &attachments);

                // Send "Delivered" receipt
                self.add_receipt_event(ReceiptEvent::new(uuid, timestamp, Receipt::Delivered));

                // Save (possibly new) expire timer
                let channel = self.get_channel(channel_idx);
                if channel.expire_timer != expire_timer {
                    tracing::info!(
                        "Expire timer changed in channel {}: {:?}",
                        channel.name,
                        expire_timer
                    );
                    channel.expire_timer = expire_timer;
                }
                let expire_timestamp = ExpireTimer::from_delay_s_opt(expire_timer);

                let quote = quote
                    .and_then(|q| Message::from_quote(q, expire_timestamp))
                    .map(Box::new);
                let message = Message {
                    quote,
                    ..Message::new(uuid, body, timestamp, attachments, expire_timestamp)
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
                            uuid: Some(sender_uuid),
                            ..
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
                .ok_or(())?;
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
                error!("Got a single typing hash set on a group.");
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
                error!("Got a single typing hash set on a group.");
            }
        }
        Ok(())
    }

    pub fn add_receipt_event(&mut self, event: ReceiptEvent) {
        self.receipt_handler.add_receipt_event(event);
    }

    fn handle_receipt(&mut self, sender_uuid: Uuid, receipt: Receipt, mut timestamps: Vec<u64>) {
        let sender_channels =
            self.data
                .channels
                .items
                .iter_mut()
                .filter(|channel| match channel.id {
                    ChannelId::User(uuid) => uuid == sender_uuid,
                    ChannelId::Group(_) => channel
                        .group_data
                        .as_ref()
                        .map(|group_data| group_data.members.contains(&sender_uuid))
                        .unwrap_or(false),
                });

        timestamps.sort_unstable_by_key(|&ts| Reverse(ts));
        if timestamps.is_empty() {
            return;
        }

        let mut is_found = false;

        for channel in sender_channels {
            let mut messages = channel.messages.items.iter_mut().rev();
            for &ts in &timestamps {
                // Note: `&mut` is needed to advance the iterator `messages` with each `ts`.
                // Since these are sorted in reverse order, we can continue advancing messages
                // without consuming them.
                if let Some(msg) = (&mut messages)
                    .take_while(|msg| msg.arrived_at >= ts)
                    .find(|msg| msg.arrived_at == ts)
                {
                    msg.receipt = msg.receipt.max(receipt);
                    is_found = true;
                }
            }

            if is_found {
                // if one ts was found, then all other ts have to be in the same channel
                self.save().unwrap();
                return;
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
            let mut notification = format!("reacted {}", emoji);
            if let Some(text) = message.message.as_ref() {
                notification.push_str(" to: ");
                notification.push_str(text);
            }

            // makes borrow checker happy
            let channel_id = channel.id;
            let channel_name = channel.name.clone();

            let sender_name = self.name_by_id(sender_uuid);
            let summary = if let ChannelId::Group(_) = channel_id {
                Cow::from(format!("{} in {}", sender_name, channel_name))
            } else {
                Cow::from(sender_name)
            };

            if notify {
                self.notify(&summary, &format!("{summary} {notification}"));
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
                    expire_timer,
                } = self.signal_manager.resolve_group(master_key).await?;

                self.ensure_users_are_known(
                    group_data
                        .members
                        .iter()
                        .copied()
                        .zip(profile_keys.into_iter()),
                )
                .await;

                let channel = self.get_channel(channel_idx);
                channel.name = name;
                channel.group_data = Some(group_data);
                channel.expire_timer = expire_timer;
            }
            Ok(channel_idx)
        } else {
            let ResolvedGroup {
                name,
                group_data,
                profile_keys,
                expire_timer,
            } = self.signal_manager.resolve_group(master_key).await?;

            self.ensure_users_are_known(
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
                expire_timer,
            });
            Ok(self.data.channels.items.len() - 1)
        }
    }

    async fn ensure_user_is_known(&mut self, uuid: Uuid, profile_key: ProfileKey) {
        // is_known <=>
        //   * in names, or
        //   * is not a phone numbers, or
        //   * is not their uuid
        let is_known = self
            .data
            .names
            .get(&uuid)
            .filter(|name| !util::is_phone_number(name) && Uuid::parse_str(name) != Ok(uuid))
            .is_some();
        if !is_known {
            if let Some(name) = self
                .signal_manager
                .contact_by_id(uuid)
                .ok()
                .flatten()
                .and_then(|c| {
                    c.address
                        .phonenumber
                        .map(|p| p.format().mode(Mode::E164).to_string())
                })
            {
                // resolved from contact list
                self.data.names.insert(uuid, name);
            } else if let Some(name) = self
                .signal_manager
                .resolve_name_from_profile(uuid, profile_key)
                .await
            {
                // resolved from signal service via their profile
                self.data.names.insert(uuid, name);
            } else {
                // failed to resolve
                self.data.names.insert(uuid, uuid.to_string());
            }
        }
    }

    async fn ensure_users_are_known(
        &mut self,
        users_with_keys: impl Iterator<Item = (Uuid, ProfileKey)>,
    ) {
        // TODO: Run in parallel
        for (uuid, profile_key) in users_with_keys {
            self.ensure_user_is_known(uuid, profile_key).await;
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
            // FIXME check whether own channel has disappearing messages enabled
            self.data.channels.items.push(Channel {
                id: user_id.into(),
                name: self.config.user.name.clone(),
                group_data: None,
                messages: StatefulList::with_items(Vec::new()),
                unread_messages: 0,
                typing: TypingSet::SingleTyping(false),
                expire_timer: None,
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
            let channel = &mut self.data.channels.items[channel_idx];
            if channel.name != name {
                channel.name = name.to_string();
            }
            channel_idx
        } else {
            let expire_timer = self
                .signal_manager
                .contact_by_id(uuid)
                .unwrap_or(None)
                .map(|c| c.expire_timer);
            self.data.channels.items.push(Channel {
                id: uuid.into(),
                name: name.to_string(),
                group_data: None,
                messages: StatefulList::with_items(Vec::new()),
                unread_messages: 0,
                typing: TypingSet::SingleTyping(false),
                expire_timer,
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

    pub fn toggle_search(&mut self) {
        self.is_searching = !self.is_searching;
    }

    pub fn is_help(&self) -> bool {
        self.display_help
    }

    pub(crate) async fn request_contacts_sync(&mut self) -> anyhow::Result<()> {
        let now = Utc::now();
        let do_sync = self
            .data
            .contacts_sync_request_at
            .map(|dt| dt + Duration::seconds(CONTACTS_SYNC_DEADLINE_SEC) < now)
            .unwrap_or(true);
        if do_sync {
            info!("requesting contact sync");
            self.signal_manager.request_contacts_sync().await?;
            self.data.contacts_sync_request_at = Some(now);
            self.save().unwrap();
        }
        Ok(())
    }

    /// Filters visible channel based on the provided `pattern`
    ///
    /// `pattern` is compared to channel name or channel member contact names, case insensitively.
    pub(crate) fn filter_channels(&mut self, pattern: &str) {
        let pattern = pattern.to_lowercase();

        // move out `channels` temporarily to make borrow checker happy
        let mut channels = std::mem::take(&mut self.data.channels);
        channels.filter(|channel: &Channel| match pattern.chars().next() {
            None => true,
            Some('@') => match channel.group_data.as_ref() {
                Some(group_data) => group_data
                    .members
                    .iter()
                    .any(|&id| self.name_by_id(id).to_lowercase().contains(&pattern[1..])),
                None => channel.name.to_lowercase().contains(&pattern[1..]),
            },
            _ => channel.name.to_lowercase().contains(&pattern),
        });
        self.data.channels = channels;
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
        *body = Some(format!("<{}>", e));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::User;
    use crate::data::GroupData;
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
