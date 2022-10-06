use std::borrow::Cow;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

pub trait Storage {
    fn channels<'s>(&'s self) -> Box<dyn Iterator<Item = Cow<Channel>> + 's>;
    fn channel(&self, channel_id: ChannelId) -> Option<Cow<Channel>>;
    fn store_channel(&mut self, channel: Channel) -> Cow<Channel>;

    fn messages<'s>(&'s self, channel_id: ChannelId)
        -> Box<dyn Iterator<Item = Cow<Message>> + 's>;
    fn message(&self, message_id: MessageId) -> Option<Cow<Message>>;
    fn store_message(&mut self, channel_id: ChannelId, message: Message) -> Cow<Message>;

    fn names<'s>(&'s self) -> Box<dyn Iterator<Item = (Uuid, Cow<str>)> + 's>;
    fn name(&self, id: Uuid) -> Option<Cow<str>>;
    fn store_name(&mut self, id: Uuid, name: String) -> Cow<str>;

    fn metadata(&self) -> Cow<Metadata>;
    fn store_metadata(&mut self, metadata: Metadata) -> Cow<Metadata>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageId {
    pub channel_id: ChannelId,
    pub arrived_at: u64,
}

impl MessageId {
    pub fn new(channel_id: ChannelId, arrived_at: u64) -> Self {
        Self {
            channel_id,
            arrived_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Metadata {
    pub contacts_sync_request_at: Option<DateTime<Utc>>,
}
