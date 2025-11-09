use std::borrow::Cow;

use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

use super::{MessageId, Metadata, Storage};

/// A storage which actually does not store anything, therefore forgetful.
pub struct ForgetfulStorage;

impl Storage for ForgetfulStorage {
    fn channels(&self) -> Box<dyn Iterator<Item = Cow<'_, Channel>> + '_> {
        Box::new(std::iter::empty())
    }

    fn channel(&self, _channel_id: ChannelId) -> Option<Cow<'_, Channel>> {
        None
    }

    fn store_channel(&mut self, channel: Channel) -> Cow<'_, Channel> {
        Cow::Owned(channel)
    }

    fn edits(
        &self,
        _message_id: MessageId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<'_, Message>> + '_> {
        Box::new(std::iter::empty())
    }

    fn message(&self, _message_id: MessageId) -> Option<Cow<'_, Message>> {
        None
    }

    fn store_message(&mut self, _channel_id: ChannelId, message: Message) -> Cow<'_, Message> {
        Cow::Owned(message)
    }

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

    fn message_channel(&self, _arrived_at: u64) -> Option<ChannelId> {
        None
    }

    fn message_id_at(&self, _channel_id: ChannelId, _idx: usize) -> Option<MessageId> {
        None
    }

    fn count_messages(&self, _channel_id: ChannelId, _after: u64) -> usize {
        0
    }
}
