use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

use super::{MessageId, Metadata, Storage};

/// A storage which actually does not store anything, therefore forgetful.
pub struct ForgetfulStorage;

impl Storage for ForgetfulStorage {
    fn channels(&self) -> Vec<Channel> {
        Vec::new()
    }

    fn channel(&self, _channel_id: ChannelId) -> Option<Channel> {
        None
    }

    fn store_channel(&mut self, _channel: &Channel) {}

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
    ) -> Box<dyn DoubleEndedIterator<Item = Message> + '_> {
        Box::new(std::iter::empty())
    }

    fn message(&self, _message_id: MessageId) -> Option<Message> {
        None
    }

    fn edits(&self, _message_id: MessageId) -> Box<dyn DoubleEndedIterator<Item = Message> + '_> {
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

    fn store_message(&mut self, _channel_id: ChannelId, _message: &Message) {}

    fn remove_message(&mut self, _message_id: MessageId) {}

    fn names(&self) -> Box<dyn Iterator<Item = (Uuid, String)> + '_> {
        Box::new(std::iter::empty())
    }

    fn name(&self, _id: Uuid) -> Option<String> {
        None
    }

    fn store_name(&mut self, _id: Uuid, _name: &str) {}

    fn metadata(&self) -> Metadata {
        Default::default()
    }

    fn store_metadata(&mut self, _metadata: &Metadata) {}

    fn save(&mut self) {}
}
