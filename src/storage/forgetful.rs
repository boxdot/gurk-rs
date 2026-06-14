use std::borrow::Cow;

use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

use super::{MessageId, Metadata, Storage};

/// A storage which actually does not store anything, therefore forgetful.
pub struct ForgetfulStorage;

impl Storage for ForgetfulStorage {
    fn channels(&self) -> Vec<Channel> {
        Vec::new()
    }

    fn channel(&self, _channel_id: ChannelId) -> Option<Cow<'_, Channel>> {
        None
    }

    fn channels_by_recency(&self) -> Vec<(ChannelId, Option<u64>)> {
        Vec::new()
    }

    fn store_channel(&mut self, channel: Channel) -> Cow<'_, Channel> {
        Cow::Owned(channel)
    }

    fn messages_tail(&self, _channel_id: ChannelId, _limit: usize) -> Vec<Message> {
        Vec::new()
    }

    fn messages_before(&self, _channel_id: ChannelId, _anchor: u64, _limit: usize) -> Vec<Message> {
        Vec::new()
    }

    fn messages_after(&self, _channel_id: ChannelId, _anchor: u64, _limit: usize) -> Vec<Message> {
        Vec::new()
    }

    fn messages(
        &self,
        _channel_id: ChannelId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<'_, Message>> + '_> {
        Box::new(std::iter::empty())
    }

    fn message(&self, _message_id: MessageId) -> Option<Cow<'_, Message>> {
        None
    }

    fn edits(
        &self,
        _message_id: MessageId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<'_, Message>> + '_> {
        Box::new(std::iter::empty())
    }

    fn messages_count_after(&self, _channel_id: ChannelId, _arrived_at: u64) -> usize {
        0
    }

    fn remove_expired(&self, _now_ms: u64) -> Vec<MessageId> {
        Vec::new()
    }

    fn next_expiring_at(&self) -> Option<u64> {
        None
    }

    fn store_message(&mut self, _channel_id: ChannelId, message: Message) -> Cow<'_, Message> {
        Cow::Owned(message)
    }

    fn remove_message(&mut self, _message_id: MessageId) {}

    fn names(&self) -> Box<dyn Iterator<Item = (Uuid, Cow<'_, str>)> + '_> {
        Box::new(std::iter::empty())
    }

    fn name(&self, _id: Uuid) -> Option<Cow<'_, str>> {
        None
    }

    fn store_name(&mut self, _id: Uuid, name: String) -> Cow<'_, str> {
        Cow::Owned(name)
    }

    fn metadata(&self) -> Cow<'_, Metadata> {
        Cow::Owned(Default::default())
    }

    fn store_metadata(&mut self, metadata: Metadata) -> Cow<'_, Metadata> {
        Cow::Owned(metadata)
    }

    fn save(&mut self) {}
}
