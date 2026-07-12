use std::borrow::Cow;
use std::cmp::Reverse;
use std::time::Instant;

use anyhow::{Context as _, anyhow};
use itertools::Itertools;
use presage::libsignal_service::content::{Content, ContentBody, Metadata};
use presage::libsignal_service::protocol::ServiceId;
use presage::proto::sync_message::{Read, Sent};
use presage::proto::{
    AttachmentPointer, DataMessage, EditMessage, ReceiptMessage, SyncMessage, TypingMessage,
};
use presage::proto::{GroupContextV2, data_message::Delete, data_message::Reaction};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::data::{BodyRange, ChannelId, Message, TypingAction, TypingSet, parse_uuid};
use crate::receipt::{Receipt, ReceiptEvent};
use crate::signal::{Attachment, GroupIdentifierBytes};
use crate::storage::MessageId;

use super::{
    App, HandleReactionOptions, add_emoji_from_sticker, notification_text_for_attachments,
};

impl App {
    /// Stores the `message` in the storage and updates the message window if the channel matches.
    pub(crate) fn store_message(&mut self, channel_id: ChannelId, message: Message) {
        self.storage.store_message(channel_id, &message);
        if let Some(window) = self.window.as_mut()
            && window.channel_id() == channel_id
        {
            window.upsert(message);
        }
    }

    /// Applies an edit to a stored message and updates the message window with the edited
    /// original.
    ///
    /// Returns `None` if the message pointed to by `target_sent_timestamp` does not exist.
    pub(crate) fn store_edited_message(
        &mut self,
        channel_id: ChannelId,
        target_sent_timestamp: u64,
        message: Message,
    ) -> Option<()> {
        let message =
            self.storage
                .store_edited_message(channel_id, target_sent_timestamp, message)?;
        if let Some(window) = self.window.as_mut()
            && window.channel_id() == channel_id
        {
            window.upsert(message);
        }
        Some(())
    }

    pub async fn on_message(&mut self, content: Box<Content>) -> anyhow::Result<()> {
        // tracing::info!(?content, "incoming");

        #[cfg(feature = "dev")]
        if self.config.developer.dump_raw_messages
            && let Err(e) = crate::dev::dump_raw_message(&content)
        {
            warn!(error = %e, "failed to dump raw message");
        }

        let user_id = self.user_id;

        if let ContentBody::SynchronizeMessage(SyncMessage { ref read, .. }) = content.body {
            self.handle_read(read);
        }

        let (channel_id, message) = match (content.metadata, content.body) {
            // Sync delete: we deleted a message from another device (any channel type)
            (
                _,
                ContentBody::SynchronizeMessage(
                    sync_message @ SyncMessage {
                        sent:
                            Some(Sent {
                                message:
                                    Some(DataMessage {
                                        delete:
                                            Some(Delete {
                                                target_sent_timestamp: Some(target_sent_timestamp),
                                            }),
                                        ..
                                    }),
                                ..
                            }),
                        ..
                    },
                ),
            ) => {
                if let Some(channel_id) = sync_message.channel_id() {
                    let message_id = MessageId::new(channel_id, target_sent_timestamp);
                    self.storage.delete_message(message_id);
                    info!(target_sent_timestamp, "message deleted via sync");
                } else {
                    warn!(
                        target_sent_timestamp,
                        "received sync delete for unknown channel"
                    );
                }
                return Ok(());
            }
            // Delete for me: message removed from our own devices only
            (
                _,
                ContentBody::SynchronizeMessage(SyncMessage {
                    delete_for_me: Some(delete_for_me),
                    ..
                }),
            ) => {
                for msg_delete in &delete_for_me.message_deletes {
                    let channel_id = msg_delete
                        .conversation
                        .as_ref()
                        .and_then(conversation_to_channel_id);
                    let Some(channel_id) = channel_id else {
                        debug!("skipping delete-for-me with unresolvable conversation");
                        continue;
                    };
                    for msg in &msg_delete.messages {
                        if let Some(ts) = msg.sent_timestamp {
                            let message_id = MessageId::new(channel_id, ts);
                            self.storage.remove_message(message_id);
                            self.remove_message_from_view(channel_id, ts);
                            info!(sent_timestamp = ts, "message removed via delete-for-me");
                        }
                    }
                }
                return Ok(());
            }
            // Private note message
            (
                _,
                ContentBody::SynchronizeMessage(SyncMessage {
                    sent:
                        Some(Sent {
                            destination_service_id: ref dest_str,
                            destination_service_id_binary: ref dest_binary,
                            timestamp: Some(timestamp),
                            message:
                                Some(DataMessage {
                                    mut body,
                                    attachments: attachment_pointers,
                                    sticker,
                                    body_ranges,
                                    reaction: None,
                                    expire_timer,
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if parse_uuid(dest_str.as_deref(), dest_binary.as_deref()) == Some(user_id) => {
                let channel_id = self.ensure_own_channel_exists();
                self.update_channel_expire_timer(channel_id, expire_timer);

                let attachments = self.save_attachments(attachment_pointers).await;
                add_emoji_from_sticker(&mut body, sticker);

                let body_ranges = body_ranges.into_iter().filter_map(BodyRange::from_proto);

                let message = Message {
                    expire_timer,
                    ..Message::new(user_id, body, body_ranges, timestamp, attachments)
                };
                (channel_id, message)
            }
            // reactions
            (
                Metadata { sender, .. },
                ContentBody::SynchronizeMessage(SyncMessage {
                    sent:
                        Some(Sent {
                            destination_service_id: ref dest_str,
                            destination_service_id_binary: ref dest_binary,
                            message:
                                Some(DataMessage {
                                    body: None,
                                    group_v2,
                                    reaction:
                                        Some(Reaction {
                                            emoji: Some(emoji),
                                            remove,
                                            target_author_aci: ref target_author_aci_str,
                                            ref target_author_aci_binary,
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
                } else if let Some(uuid) = parse_uuid(dest_str.as_deref(), dest_binary.as_deref()) {
                    ChannelId::User(uuid)
                } else {
                    let uuid = parse_uuid(
                        target_author_aci_str.as_deref(),
                        target_author_aci_binary.as_deref(),
                    )
                    .context("missing target author ACI in sync reaction")?;
                    ChannelId::User(uuid)
                };

                let channel_muted = self
                    .storage
                    .channel(channel_id)
                    .map(|c| c.muted)
                    .unwrap_or(false);
                self.handle_reaction(
                    channel_id,
                    target_sent_timestamp,
                    sender.raw_uuid(),
                    emoji,
                    HandleReactionOptions::new()
                        .remove(remove.unwrap_or(false))
                        .notify(self.config.notifications.show_reactions && !channel_muted)
                        .bell(!self.config.notifications.mute_reactions_bell && !channel_muted),
                )
                .await;
                read.into_iter().for_each(|r| {
                    self.handle_receipt(
                        r.parse_sender_aci().map(Into::into).unwrap_or_default(),
                        Receipt::Read,
                        vec![r.timestamp.unwrap()],
                    );
                });
                return Ok(());
            }
            (
                Metadata { sender, .. },
                ContentBody::DataMessage(DataMessage {
                    body: None,
                    group_v2,
                    reaction:
                        Some(Reaction {
                            emoji: Some(emoji),
                            remove,
                            target_sent_timestamp: Some(target_sent_timestamp),
                            target_author_aci: ref target_author_aci_str,
                            ref target_author_aci_binary,
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
                } else if sender.raw_uuid() == self.user_id {
                    // reaction from us => target author is the user channel
                    let uuid = parse_uuid(
                        target_author_aci_str.as_deref(),
                        target_author_aci_binary.as_deref(),
                    )
                    .context("missing target author ACI in reaction")?;
                    ChannelId::User(uuid)
                } else {
                    // reaction is from somebody else => they are the user channel
                    ChannelId::User(sender.raw_uuid())
                };

                let channel_muted = self
                    .storage
                    .channel(channel_id)
                    .map(|c| c.muted)
                    .unwrap_or(false);
                self.handle_reaction(
                    channel_id,
                    target_sent_timestamp,
                    sender.raw_uuid(),
                    emoji,
                    HandleReactionOptions::new()
                        .remove(remove.unwrap_or(false))
                        .notify(self.config.notifications.show_reactions && !channel_muted)
                        .bell(!self.config.notifications.mute_reactions_bell && !channel_muted),
                )
                .await;
                return Ok(());
            }
            // Direct/group message by us from a different device
            (
                Metadata { sender, .. },
                ContentBody::SynchronizeMessage(SyncMessage {
                    sent:
                        Some(Sent {
                            destination_service_id: ref dest_str,
                            destination_service_id_binary: ref dest_binary,
                            timestamp: Some(timestamp),
                            message:
                                Some(DataMessage {
                                    mut body,
                                    profile_key,
                                    group_v2,
                                    quote,
                                    attachments: attachment_pointers,
                                    sticker,
                                    body_ranges,
                                    reaction: None,
                                    expire_timer,
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if sender.raw_uuid() == user_id => {
                let channel_id = if let Some(GroupContextV2 {
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
                } else if let Some(destination_uuid) =
                    parse_uuid(dest_str.as_deref(), dest_binary.as_deref())
                {
                    let profile_key = profile_key
                        .context("sync message with destination without profile key")?
                        .try_into()
                        .map_err(|_| anyhow!("invalid profile key"))?;
                    let name = self.name_by_id(destination_uuid).await;
                    self.ensure_user_is_known(destination_uuid, Some(profile_key))
                        .await;
                    self.ensure_contact_channel_exists(destination_uuid, &name)
                        .await
                } else {
                    debug!("dropping a sync message not attached to a channel");
                    return Ok(());
                };

                add_emoji_from_sticker(&mut body, sticker);

                // Update channel's expire timer if it changed
                self.update_channel_expire_timer(channel_id, expire_timer);

                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let attachments = self.save_attachments(attachment_pointers).await;
                let body_ranges = body_ranges.into_iter().filter_map(BodyRange::from_proto);

                let message = Message {
                    quote,
                    expire_timer,
                    ..Message::new(user_id, body, body_ranges, timestamp, attachments)
                };

                if message.is_empty() {
                    debug!("dropping empty message");
                    return Ok(());
                }

                (channel_id, message)
            }
            // Incoming remote delete (delete for everyone)
            (
                Metadata { sender, .. },
                ContentBody::DataMessage(DataMessage {
                    delete:
                        Some(Delete {
                            target_sent_timestamp: Some(target_sent_timestamp),
                        }),
                    group_v2,
                    ..
                }),
            ) => {
                let channel_id = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    let master_key = master_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid group master key"))?;
                    self.ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?
                } else {
                    let name = self.name_by_id(sender.raw_uuid()).await;
                    self.ensure_contact_channel_exists(sender.raw_uuid(), &name)
                        .await
                };

                let message_id = MessageId::new(channel_id, target_sent_timestamp);
                if self.storage.delete_message(message_id) {
                    info!(target_sent_timestamp, "message deleted remotely");
                } else {
                    warn!(
                        target_sent_timestamp,
                        "received remote delete for unknown message"
                    );
                }
                return Ok(());
            }
            // Incoming direct/group message
            (
                Metadata { sender, .. },
                ContentBody::DataMessage(DataMessage {
                    mut body,
                    group_v2,
                    timestamp: Some(timestamp),
                    profile_key,
                    quote,
                    attachments: attachment_pointers,
                    sticker,
                    body_ranges,
                    expire_timer,
                    ..
                }),
            ) => {
                let (channel_id, from, channel_muted) = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    // incoming group message
                    // profile_key can be None and is not required for known contacts
                    let profile_key = match profile_key {
                        Some(profile_key) => Some(
                            profile_key
                                .try_into()
                                .map_err(|_| anyhow!("invalid profile key"))?,
                        ),
                        None => None,
                    };
                    let master_key = master_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid group master key"))?;
                    let channel_id = self
                        .ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?;

                    self.ensure_user_is_known(sender.raw_uuid(), profile_key)
                        .await;
                    let from = self.name_by_id(sender.raw_uuid()).await;
                    let channel = self.channel(channel_id).expect("non-existent channel");
                    (channel_id, from, channel.muted)
                } else {
                    // incoming direct message
                    let profile_key = profile_key
                        .context("sync message with destination without profile key")?
                        .try_into()
                        .map_err(|_| anyhow!("invalid profile key"))?;
                    self.ensure_user_is_known(sender.raw_uuid(), Some(profile_key))
                        .await;
                    let name = self.name_by_id(sender.raw_uuid()).await;
                    let channel_id = self
                        .ensure_contact_channel_exists(sender.raw_uuid(), &name)
                        .await;
                    // Reset typing notification as the Tipyng::Stop are not always sent by the server when a message is sent.
                    let (from, channel_muted) = {
                        let channel = self.channel(channel_id).expect("non-existent channel");
                        (channel.name.clone(), channel.muted)
                    };
                    self.channels.modify_channel_by_id(
                        &mut *self.storage,
                        channel_id,
                        |channel| channel.reset_writing(sender.raw_uuid()),
                    );
                    (channel_id, from, channel_muted)
                };

                add_emoji_from_sticker(&mut body, sticker);

                // Update channel's expire timer if it changed
                self.update_channel_expire_timer(channel_id, expire_timer);

                let attachments = self.save_attachments(attachment_pointers).await;
                if !channel_muted {
                    self.notify_about_message(&from, body.as_deref(), &attachments);
                }

                // Send "Delivered" receipt
                self.add_receipt_event(ReceiptEvent::new(
                    sender.raw_uuid(),
                    timestamp,
                    Receipt::Delivered,
                ));

                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let body_ranges = body_ranges.into_iter().filter_map(BodyRange::from_proto);
                let message = Message {
                    quote,
                    expire_timer,
                    ..Message::new(sender.raw_uuid(), body, body_ranges, timestamp, attachments)
                };

                if message.is_empty() {
                    return Ok(());
                }

                (channel_id, message)
            }
            (metadata, ContentBody::SynchronizeMessage(sync_message)) => {
                return self.handle_sync_message(metadata, sync_message);
            }
            (
                Metadata { sender, .. },
                ContentBody::ReceiptMessage(ReceiptMessage {
                    r#type: Some(receipt_type),
                    timestamp: timestamps,
                }),
            ) => {
                let receipt = Receipt::from_i32(receipt_type);
                self.handle_receipt(sender.raw_uuid(), receipt, timestamps);
                return Ok(());
            }

            (
                Metadata { sender, .. },
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
                        sender.raw_uuid(),
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

            // Incoming edit from another user
            (
                Metadata { sender, .. },
                ContentBody::EditMessage(EditMessage {
                    target_sent_timestamp: Some(target_sent_timestamp),
                    data_message:
                        Some(DataMessage {
                            mut body,
                            group_v2,
                            timestamp: Some(timestamp),
                            profile_key,
                            body_ranges,
                            sticker,
                            ..
                        }),
                }),
            ) => {
                let channel_id = if let Some(GroupContextV2 {
                    master_key: Some(master_key),
                    revision: Some(revision),
                    ..
                }) = group_v2
                {
                    let profile_key = match profile_key {
                        Some(pk) => {
                            Some(pk.try_into().map_err(|_| anyhow!("invalid profile key"))?)
                        }
                        None => None,
                    };
                    let master_key = master_key
                        .try_into()
                        .map_err(|_| anyhow!("invalid group master key"))?;
                    let channel_id = self
                        .ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?;
                    self.ensure_user_is_known(sender.raw_uuid(), profile_key)
                        .await;
                    channel_id
                } else {
                    let profile_key = profile_key.and_then(|pk| pk.try_into().ok());
                    self.ensure_user_is_known(sender.raw_uuid(), profile_key)
                        .await;
                    let name = self.name_by_id(sender.raw_uuid()).await;
                    self.ensure_contact_channel_exists(sender.raw_uuid(), &name)
                        .await
                };

                add_emoji_from_sticker(&mut body, sticker);
                let body_ranges = body_ranges.into_iter().filter_map(BodyRange::from_proto);
                let message = Message::new(sender.raw_uuid(), body, body_ranges, timestamp, vec![]);

                if self
                    .store_edited_message(channel_id, target_sent_timestamp, message)
                    .is_some()
                {
                    self.touch_channel(channel_id, sender.raw_uuid() == self.user_id);
                } else {
                    warn!(
                        target_sent_timestamp,
                        "could not find original message to apply edit"
                    );
                }
                return Ok(());
            }

            unhandled => {
                info!(?unhandled, "skipping unhandled message");
                return Ok(());
            }
        };

        self.add_message_to_channel(channel_id, message);

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
        self.bell();
    }

    fn update_channel_expire_timer(&mut self, channel_id: ChannelId, expire_timer: Option<u32>) {
        let new_timer = expire_timer.filter(|&t| t > 0);
        self.channels
            .modify_channel_by_id(&mut *self.storage, channel_id, |channel| {
                if channel.expire_timer != new_timer {
                    channel.expire_timer = new_timer;
                    true
                } else {
                    false
                }
            });
    }

    pub fn step_receipts(&mut self) {
        self.receipt_handler.step(self.signal_manager.as_ref());
    }

    /// Expire typing indicators older than TYPING_TIMEOUT_SECS
    pub fn expire_typing_indicators(&mut self) {
        // Can't iterate through channels and update them at the same time
        let writing: Vec<ChannelId> = self
            .channels()
            .iter()
            .filter(|channel| channel.is_writing())
            .map(|channel| channel.id)
            .collect();
        for channel_id in writing {
            self.channels
                .modify_channel_by_id(&mut *self.storage, channel_id, |channel| {
                    channel.expire_typing()
                });
        }
    }

    fn handle_typing(
        &mut self,
        sender_uuid: Uuid,
        group_id: Option<GroupIdentifierBytes>,
        action: TypingAction,
        _timestamp: u64,
    ) -> Result<(), ()> {
        if let Some(gid) = group_id {
            self.channels
                .modify_channel_by_id(&mut *self.storage, ChannelId::Group(gid), |channel| {
                    if let TypingSet::GroupTyping(ref mut map) = channel.typing {
                        match action {
                            TypingAction::Started => {
                                map.insert(sender_uuid, Instant::now());
                            }
                            TypingAction::Stopped => {
                                map.remove(&sender_uuid);
                            }
                        }
                        true
                    } else {
                        error!("Got a single typing instead of hash set on a group");
                        false
                    }
                });
        } else {
            self.channels.modify_channel_by_id(
                &mut *self.storage,
                ChannelId::User(sender_uuid),
                |channel| {
                    if let TypingSet::SingleTyping(_) = channel.typing {
                        match action {
                            TypingAction::Started => {
                                channel.typing = TypingSet::SingleTyping(Some(Instant::now()));
                            }
                            TypingAction::Stopped => {
                                channel.typing = TypingSet::SingleTyping(None);
                            }
                        }
                        true
                    } else {
                        error!("Got a hash set instead of single typing on a direct chat");
                        false
                    }
                },
            );
        }
        Ok(())
    }

    pub fn add_receipt_event(&mut self, event: ReceiptEvent) {
        self.receipt_handler.add_receipt_event(event);
    }

    fn handle_receipt(&mut self, sender_uuid: Uuid, receipt: Receipt, mut timestamps: Vec<u64>) {
        let sender_channels: Vec<ChannelId> = self
            .channels()
            .iter()
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
                    let mut msg = msg;
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
                self.store_message(channel_id, message);
            }
        }
    }

    pub(super) async fn handle_reaction(
        &mut self,
        channel_id: ChannelId,
        target_sent_timestamp: u64,
        sender_uuid: Uuid,
        emoji: String,
        HandleReactionOptions {
            remove,
            notify,
            bell,
        }: HandleReactionOptions,
    ) -> Option<()> {
        let mut message = self
            .storage
            .message(MessageId::new(channel_id, target_sent_timestamp))?;
        let from_current_user = self.user_id == message.from_id;

        let reaction_idx = message
            .reactions
            .iter()
            .position(|(from_id, _)| from_id == &sender_uuid);
        let is_added = if let Some(idx) = reaction_idx {
            if remove {
                message.reactions.swap_remove(idx);
                false
            } else {
                message.reactions[idx].1.clone_from(&emoji);
                true
            }
        } else {
            message.reactions.push((sender_uuid, emoji.clone()));
            true
        };
        let message_text = message.message.clone();
        self.store_message(channel_id, message);

        if is_added && channel_id != ChannelId::User(self.user_id) {
            // Notification
            let mut notification = format!("reacted {emoji}");
            if let Some(text) = message_text.as_ref() {
                notification.push_str(" to: ");
                notification.push_str(text);
            }

            // makes borrow checker happy
            let channel = self.storage.channel(channel_id)?;
            let channel_name = channel.name.clone();

            let sender_name = self.name_by_id(sender_uuid).await;
            let summary = if let ChannelId::Group(_) = channel_id {
                Cow::from(format!("{sender_name} in {channel_name}"))
            } else {
                Cow::from(sender_name)
            };

            if notify {
                self.notify(&summary, &format!("{summary} {notification}"));
            }

            if bell {
                self.bell();
            }

            self.touch_channel(channel_id, from_current_user);
        }

        Some(())
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

    fn notify(&self, summary: &str, text: &str) {
        if self.config.notifications.enabled
            && let Err(e) = notify_rust::Notification::new()
                .summary(if self.config.notifications.show_message_chat {
                    summary
                } else {
                    "gurk"
                })
                .body(if self.config.notifications.show_message_text {
                    text
                } else {
                    "New message!"
                })
                .show()
        {
            error!("failed to send notification: {}", e);
        }
    }

    fn bell(&self) {
        if self.config.bell {
            print!("\x07");
        }
    }

    // Absorbed from handlers.rs

    pub(super) fn handle_sync_message(
        &mut self,
        metadata: Metadata,
        sync_message: SyncMessage,
    ) -> anyhow::Result<()> {
        let Some(channel_id) = sync_message.channel_id() else {
            debug!("dropping a sync message not attached to a channel");
            return Ok(());
        };

        // edit message
        if let Some(Sent {
            edit_message:
                Some(EditMessage {
                    target_sent_timestamp: Some(target_sent_timestamp),
                    data_message:
                        Some(DataMessage {
                            body: Some(body),
                            timestamp: Some(arrived_at),
                            ..
                        }),
                }),
            ..
        }) = sync_message.sent
        {
            let from_id = metadata.sender.raw_uuid();
            self.store_edited_message(
                channel_id,
                target_sent_timestamp,
                Message::text(from_id, arrived_at, body),
            )
            .context("no message to edit")?;

            self.touch_channel(channel_id, from_id == self.user_id);
        }

        Ok(())
    }

    /// Handles read notifications
    pub(crate) fn handle_read(&mut self, read: &[Read]) {
        if read.is_empty() {
            return;
        }

        let mut read_at: Vec<_> = read.iter().filter_map(|read| read.timestamp).collect();
        read_at.sort_unstable();

        let mut updates = Vec::new();
        for channel in self.channels() {
            // skip channels without unread messages
            if channel.unread_messages == 0 {
                continue;
            }

            // find the last read message in this channel
            let Some(last_read_at) = read_at.iter().rev().copied().find(|&timestamp| {
                self.storage
                    .message(MessageId::new(channel.id, timestamp))
                    .is_some()
            }) else {
                continue;
            };

            let num_unread = self.storage.messages_count_after(channel.id, last_read_at);
            updates.push((channel.id, (num_unread as u32).min(channel.unread_messages)));
        }

        // Can't iterate though channels and update them at the same time
        for (channel_id, unread_messages) in updates {
            self.channels
                .modify_channel_by_id(&mut *self.storage, channel_id, |channel| {
                    channel.unread_messages = unread_messages;
                    true
                });
        }
    }
}

trait MessageExt {
    /// Get a channel id a message
    fn channel_id(&self) -> Option<ChannelId>;
}

impl MessageExt for SyncMessage {
    fn channel_id(&self) -> Option<ChannelId> {
        // only sent sync message are attached to a conversation
        let sent = self.sent.as_ref()?;
        if let Some(uuid) = sent.parse_destination_uuid() {
            Some(ChannelId::User(uuid))
        } else {
            let group_v2 = sent
                .message
                .as_ref()
                .and_then(|message| message.group_v2.as_ref())
                .or_else(|| {
                    sent.edit_message
                        .as_ref()?
                        .data_message
                        .as_ref()?
                        .group_v2
                        .as_ref()
                })?;
            ChannelId::from_master_key_bytes(group_v2.master_key.as_deref()?).ok()
        }
    }
}

trait SyncSentExt {
    fn parse_destination_uuid(&self) -> Option<Uuid>;
}

impl SyncSentExt for Sent {
    fn parse_destination_uuid(&self) -> Option<Uuid> {
        parse_uuid(
            self.destination_service_id.as_deref(),
            self.destination_service_id_binary.as_deref(),
        )
    }
}

fn conversation_to_channel_id(conv: &presage::proto::ConversationIdentifier) -> Option<ChannelId> {
    use presage::proto::conversation_identifier::Identifier;
    match conv.identifier.as_ref()? {
        Identifier::ThreadServiceId(s) => {
            let uuid: Uuid = s.parse().ok()?;
            Some(ChannelId::User(uuid))
        }
        Identifier::ThreadServiceIdBinary(b) => {
            let sid = ServiceId::parse_from_service_id_binary(b)?;
            Some(ChannelId::User(sid.raw_uuid()))
        }
        Identifier::ThreadGroupId(b) => {
            let bytes: [u8; 32] = b.as_slice().try_into().ok()?;
            Some(ChannelId::Group(bytes))
        }
        Identifier::ThreadE164(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::app::tests::test_app;
    use crate::data::Channel;

    use super::*;

    /// Overwrites the unread counter of the given channel in memory and storage.
    fn set_unread(app: &mut App, channel_id: ChannelId, unread_messages: u32) {
        app.channels
            .modify_channel_by_id(&mut *app.storage, channel_id, |channel| {
                channel.unread_messages = unread_messages;
                true
            });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_read() {
        let (mut app, _events, _sent_messages) = test_app().await;

        // fixture channel already has "First message" at arrived_at 0
        let channel_id = app.channels().first().unwrap().id;
        app.storage.store_message(
            channel_id,
            &Message::text(app.user_id, 42, "unread message".to_string()),
        );
        set_unread(&mut app, channel_id, 2);

        // reading the older message leaves the newer one unread
        app.handle_read(&[Read {
            timestamp: Some(0),
            ..Default::default()
        }]);
        assert_eq!(app.channel(channel_id).unwrap().unread_messages, 1);

        // reading the newer message clears the counter
        app.handle_read(&[Read {
            timestamp: Some(42),
            ..Default::default()
        }]);
        assert_eq!(app.channel(channel_id).unwrap().unread_messages, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_read_only_affects_matching_channel() {
        let (mut app, _events, _sent_messages) = test_app().await;

        let channel_a = app.channels().first().unwrap().id;
        app.storage
            .store_message(channel_a, &Message::text(app.user_id, 100, "a".to_string()));
        set_unread(&mut app, channel_a, 1);

        let channel_b = ChannelId::User(Uuid::new_v4());
        app.store_channel(Channel {
            id: channel_b,
            name: "other".to_string(),
            group_data: None,
            unread_messages: 1,
            muted: false,
            typing: TypingSet::new(false),
            expire_timer: None,
        });
        app.storage
            .store_message(channel_b, &Message::text(app.user_id, 200, "b".to_string()));

        // a read receipt for channel A's message must not touch channel B
        app.handle_read(&[Read {
            timestamp: Some(100),
            ..Default::default()
        }]);
        assert_eq!(app.channel(channel_a).unwrap().unread_messages, 0);
        assert_eq!(app.channel(channel_b).unwrap().unread_messages, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_incoming_message_increments_unread_in_channel_list() {
        let (mut app, _events, _sent_messages) = test_app().await;

        // channel A is the selected one
        let channel_a = app.channels().first().unwrap().id;
        assert_eq!(app.selected_channel_id(), Some(channel_a));

        // a second, unselected channel
        let other = Uuid::new_v4();
        let channel_b = ChannelId::User(other);
        app.store_channel(Channel {
            id: channel_b,
            name: "other".to_string(),
            group_data: None,
            unread_messages: 0,
            muted: false,
            typing: TypingSet::new(false),
            expire_timer: None,
        });

        app.add_message_to_channel(channel_b, Message::text(other, 1000, "hi".to_string()));
        assert_eq!(app.channel(channel_b).unwrap().unread_messages, 1);

        app.add_message_to_channel(channel_b, Message::text(other, 1001, "there".to_string()));
        assert_eq!(app.channel(channel_b).unwrap().unread_messages, 2);
    }
}
