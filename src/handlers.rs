use std::collections::BTreeMap;

use anyhow::Context;
use presage::libsignal_service::content::Metadata;
use presage::proto::sync_message::{Read, Sent};
use presage::proto::{DataMessage, EditMessage, SyncMessage};
use tracing::debug;

use crate::app::App;
use crate::data::{ChannelId, Message};
use crate::storage::MessageId;

impl App {
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
