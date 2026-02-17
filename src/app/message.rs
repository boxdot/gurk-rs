use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::BTreeMap;

use anyhow::{Context as _, anyhow};
use itertools::Itertools;
use presage::libsignal_service::content::{Content, ContentBody, Metadata};
use presage::proto::sync_message::{Read, Sent};
use presage::proto::{
    AttachmentPointer, DataMessage, EditMessage, ReceiptMessage, SyncMessage, TypingMessage,
};
use presage::proto::{GroupContextV2, data_message::Reaction};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::data::{BodyRange, ChannelId, Message, TypingAction, TypingSet};
use crate::receipt::{Receipt, ReceiptEvent};
use crate::signal::{Attachment, GroupIdentifierBytes};
use crate::storage::MessageId;

use super::{
    App, HandleReactionOptions, add_emoji_from_sticker, notification_text_for_attachments,
};

impl App {
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

        let (channel_idx, message) = match (content.metadata, content.body) {
            // Private note message
            (
                _,
                ContentBody::SynchronizeMessage(SyncMessage {
                    sent:
                        Some(Sent {
                            destination_service_id: Some(destination_uuid),
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
            // reactions
            (
                Metadata { sender, .. },
                ContentBody::SynchronizeMessage(SyncMessage {
                    sent:
                        Some(Sent {
                            destination_service_id: destination_uuid,
                            message:
                                Some(DataMessage {
                                    body: None,
                                    group_v2,
                                    reaction:
                                        Some(Reaction {
                                            emoji: Some(emoji),
                                            remove,
                                            target_author_aci: Some(target_author_uuid),
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
                    sender.raw_uuid(),
                    emoji,
                    HandleReactionOptions::new()
                        .remove(remove.unwrap_or(false))
                        .notify(self.config.notifications.show_reactions)
                        .bell(!self.config.notifications.mute_reactions_bell),
                )
                .await;
                read.into_iter().for_each(|r| {
                    self.handle_receipt(
                        Uuid::parse_str(r.sender_aci.unwrap().as_str()).unwrap(),
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
                            target_author_aci: Some(target_author_uuid),
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
                    ChannelId::User(target_author_uuid.parse()?)
                } else {
                    // reaction is from somebody else => they are the user channel
                    ChannelId::User(sender.raw_uuid())
                };

                self.handle_reaction(
                    channel_id,
                    target_sent_timestamp,
                    sender.raw_uuid(),
                    emoji,
                    HandleReactionOptions::new()
                        .remove(remove.unwrap_or(false))
                        .notify(self.config.notifications.show_reactions)
                        .bell(!self.config.notifications.mute_reactions_bell),
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
                            destination_service_id: destination_uuid,
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
                                    ..
                                }),
                            ..
                        }),
                    ..
                }),
            ) if sender.raw_uuid() == user_id => {
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
                        .context("sync message with destination without profile key")?
                        .try_into()
                        .map_err(|_| anyhow!("invalid profile key"))?;
                    let destination_uuid = destination_uuid.parse()?;
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
                let quote = quote.and_then(Message::from_quote).map(Box::new);
                let attachments = self.save_attachments(attachment_pointers).await;
                let body_ranges = body_ranges.into_iter().filter_map(BodyRange::from_proto);

                let message = Message {
                    quote,
                    ..Message::new(user_id, body, body_ranges, timestamp, attachments)
                };

                if message.is_empty() {
                    debug!("dropping empty message");
                    return Ok(());
                }

                (channel_idx, message)
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
                    let channel_idx = self
                        .ensure_group_channel_exists(master_key, revision)
                        .await
                        .context("failed to create group channel")?;

                    self.ensure_user_is_known(sender.raw_uuid(), profile_key)
                        .await;
                    let from = self.name_by_id(sender.raw_uuid()).await;

                    (channel_idx, from)
                } else {
                    // incoming direct message
                    let profile_key = profile_key
                        .context("sync message with destination without profile key")?
                        .try_into()
                        .map_err(|_| anyhow!("invalid profile key"))?;
                    self.ensure_user_is_known(sender.raw_uuid(), Some(profile_key))
                        .await;
                    let name = self.name_by_id(sender.raw_uuid()).await;
                    let channel_idx = self
                        .ensure_contact_channel_exists(sender.raw_uuid(), &name)
                        .await;
                    // Reset typing notification as the Tipyng::Stop are not always sent by the server when a message is sent.
                    let channel_id = self.channels.items[channel_idx];
                    let mut channel = self
                        .storage
                        .channel(channel_id)
                        .expect("non-existent channel")
                        .into_owned();
                    let from = channel.name.clone();
                    if channel.reset_writing(sender.raw_uuid()) {
                        self.storage.store_channel(channel);
                    }
                    (channel_idx, from)
                };

                add_emoji_from_sticker(&mut body, sticker);

                let attachments = self.save_attachments(attachment_pointers).await;
                self.notify_about_message(&from, body.as_deref(), &attachments);

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
                    ..Message::new(sender.raw_uuid(), body, body_ranges, timestamp, attachments)
                };

                if message.is_empty() {
                    return Ok(());
                }

                (channel_idx, message)
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

            unhandled => {
                info!(?unhandled, "skipping unhandled message");
                return Ok(());
            }
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
        self.bell();
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
            .message(MessageId::new(channel_id, target_sent_timestamp))?
            .into_owned();
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

            let channel_idx = self
                .channels
                .items
                .iter()
                .position(|id| id == &channel_id)
                .expect("non-existent channel");
            self.touch_channel(channel_idx, from_current_user);
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
            // Note: target_sent_timestamp points to the previous edit or the original message
            let edited = self
                .storage
                .message(MessageId::new(channel_id, target_sent_timestamp))
                .context("no message to edit")?;

            // get original message
            let mut original = if let Some(arrived_at) = edited.edit {
                // previous edit => get original message
                self.storage
                    .message(MessageId::new(channel_id, arrived_at))
                    .context("no original edited message")?
                    .into_owned()
            } else {
                // original message => first edit
                let original = edited.into_owned();

                // preserve body of the original message; it is replaced below
                let mut preserved = original.clone();
                preserved.arrived_at = original.arrived_at + 1;
                preserved.edit = Some(original.arrived_at);
                self.storage.store_message(channel_id, preserved);

                original
            };

            // store the incoming edit
            self.storage.store_message(
                channel_id,
                Message {
                    edit: Some(original.arrived_at),
                    ..Message::text(from_id, arrived_at, body.clone())
                },
            );

            // override the body of the original message
            original.message.replace(body);
            original.edited = true;
            self.storage.store_message(channel_id, original);

            let channel_idx = self
                .channels
                .items
                .iter()
                .position(|id| id == &channel_id)
                .context("editing message in non-existent channel")?;
            self.touch_channel(channel_idx, from_id == self.user_id);
        }

        Ok(())
    }

    /// Handles read notifications
    pub(crate) fn handle_read(&mut self, read: &[Read]) {
        // First collect all the read counters to avoid hitting the storage for the same channel
        let read_counters: BTreeMap<ChannelId, u32> = read
            .iter()
            .filter_map(|read| {
                let arrived_at = read.timestamp?;
                let channel_id = self.storage.message_channel(arrived_at)?;
                let num_unread = self
                    .storage
                    .messages(channel_id)
                    .rev()
                    .take_while(|msg| arrived_at < msg.arrived_at)
                    .count();
                let num_unread: u32 = num_unread.try_into().ok()?;
                Some((channel_id, num_unread))
            })
            .collect();
        // Update the unread counters
        for (channel_id, num_unread) in read_counters {
            if let Some(channel) = self.storage.channel(channel_id)
                && channel.unread_messages > 0
            {
                let mut channel = channel.into_owned();
                channel.unread_messages = num_unread;
                self.storage.store_channel(channel);
            }
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
        if let Some(uuid) = sent
            .destination_service_id
            .as_ref()
            .and_then(|id| id.parse().ok())
        {
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

#[cfg(test)]
mod tests {
    use crate::app::tests::test_app;

    use super::*;

    #[test]
    #[ignore = "forgetful storage does not support lookup by arrived_at"]
    fn test_handle_read() {
        let (mut app, _events, _sent_messages) = test_app();

        let channel_id = *app.channels.items.first().unwrap();

        // new incoming message
        let message = app
            .storage
            .store_message(
                channel_id,
                Message::text(app.user_id, 42, "unread message".to_string()),
            )
            .into_owned();

        // mark as unread
        app.storage
            .channel(channel_id)
            .unwrap()
            .into_owned()
            .unread_messages = 1;

        app.handle_read(&[Read {
            timestamp: Some(message.arrived_at),
            ..Default::default()
        }]);

        assert_eq!(app.storage.channel(channel_id).unwrap().unread_messages, 0);
    }
}
