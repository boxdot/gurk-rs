use std::borrow::Cow;

use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

use super::{MessageId, Metadata, Storage};

/// A storage which actually does not store anything, therefore forgetful.
pub struct ForgetfulStorage;

impl Storage for ForgetfulStorage {
    fn channels<'s>(&'s self) -> Box<dyn Iterator<Item = Cow<Channel>> + 's> {
        Box::new(std::iter::empty())
    }

    fn channel(&self, _channel_id: ChannelId) -> Option<Cow<Channel>> {
        None
    }

    fn store_channel(&mut self, channel: Channel) -> Cow<Channel> {
        Cow::Owned(channel)
    }

    fn messages<'s>(
        &'s self,
        _channel_id: ChannelId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + 's> {
        Box::new(std::iter::empty())
    }

    fn message(&self, _message_id: MessageId) -> Option<Cow<Message>> {
        None
    }

    fn store_message(&mut self, _channel_id: ChannelId, message: Message) -> Cow<Message> {
        Cow::Owned(message)
    }

    fn names<'s>(&'s self) -> Box<dyn Iterator<Item = (Uuid, Cow<str>)> + 's> {
        Box::new(std::iter::empty())
    }

    fn name(&self, _id: Uuid) -> Option<Cow<str>> {
        None
    }

    fn store_name(&mut self, _id: Uuid, name: String) -> Cow<str> {
        Cow::Owned(name)
    }

    fn metadata(&self) -> Cow<Metadata> {
        Cow::Owned(Default::default())
    }

    fn store_metadata(&mut self, metadata: Metadata) -> Cow<Metadata> {
        Cow::Owned(metadata)
    }

    fn save(&mut self) {}
}
