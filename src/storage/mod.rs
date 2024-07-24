mod copy;
mod forgetful;
mod json;
mod memcache;
mod sql;

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

pub use copy::{copy, sync_from_signal};
pub use forgetful::ForgetfulStorage;
pub use json::JsonStorage;
pub use memcache::MemCache;
pub use sql::SqliteStorage;

/// Storage of channels, messages, names and metadata.
///
/// Used to persist the data to disk.
///
/// ## Implementation note
///
/// The implementer can choose whether to return owning or
/// borrowed objects from the storage. This depends whether the objects are stored as is, or are
/// converted and/or serialized.
pub trait Storage {
    /// Channels in no particular order
    fn channels(&self) -> Box<dyn Iterator<Item = Cow<Channel>> + '_>;
    /// Gets the channel by id
    fn channel(&self, channel_id: ChannelId) -> Option<Cow<Channel>>;
    /// Stores the given `channel` and returns it back
    fn store_channel(&mut self, channel: Channel) -> Cow<Channel>;

    /// Messages sorted by arrived_at in ascending order
    ///
    /// No edited messages must be included.
    fn messages(
        &self,
        channel_id: ChannelId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + '_>;
    /// Gets the message by id
    fn message(&self, message_id: MessageId) -> Option<Cow<Message>>;

    fn edits(
        &self,
        message_id: MessageId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + '_>;

    /// Stores the message for the given `channel_id` and returns it back
    ///
    /// If a channel with this `channel_id` already exists in the storage, it is overridden.
    /// Otherwise, the channel is added to the storage.
    fn store_message(&mut self, channel_id: ChannelId, message: Message) -> Cow<Message>;

    fn store_edited_message(
        &mut self,
        channel_id: ChannelId,
        target_sent_timestampt: u64,
        message: Message,
    ) -> Option<Cow<Message>> {
        // Note: target_sent_timestamp points to the previous edit or the original message
        let prev_edited = self.message(MessageId::new(channel_id, target_sent_timestampt))?;

        // get original message
        let mut original = if let Some(arrived_at) = prev_edited.edit {
            // previous edit => get original message
            self.message(MessageId::new(channel_id, arrived_at))?
                .into_owned()
        } else {
            // original message => first edit
            let original = prev_edited.into_owned();

            // preserve body of the original message; it is replaced below
            let mut preserved = original.clone();
            preserved.arrived_at = original.arrived_at + 1;
            preserved.edit = Some(original.arrived_at);
            self.store_message(channel_id, preserved);

            original
        };

        // store the incoming edit
        let body = message.message.clone();
        self.store_message(
            channel_id,
            Message {
                edit: Some(original.arrived_at),
                ..message
            },
        );

        // override the body of the original message
        original.message = body;
        original.edited = true;
        Some(self.store_message(channel_id, original))
    }

    /// Names of contacts
    fn names(&self) -> Box<dyn Iterator<Item = (Uuid, Cow<str>)> + '_>;
    /// Gets the name for the given contact `id`
    fn name(&self, id: Uuid) -> Option<Cow<str>>;
    /// Stores a name for the given contact `id`
    ///
    /// If the name with this `id` already exists in the storage, it is overridden. Otherwise, it
    /// the name is added to the storage.
    fn store_name(&mut self, id: Uuid, name: String) -> Cow<str>;

    /// Returns the metadata containing persisted flags and settings
    fn metadata(&self) -> Cow<Metadata>;
    /// Stores the new metadata in the storage overriding the previous one
    fn store_metadata(&mut self, metadata: Metadata) -> Cow<Metadata>;

    /// Persists the data in the storage
    ///
    /// ## Implementation note
    ///
    /// The implementers of this trait, can persist for each store call, if it is efficient enough.
    /// This methods must guarantee that the data is persisted in any case.
    fn save(&mut self);

    /// Returns `true` if this storage does not contains any channels and no names
    fn is_empty(&self) -> bool {
        self.channels().next().is_none() && self.names().next().is_none()
    }
}

/// A message is identified by its channel and time of arrived in milliseconds
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

/// Persisted metadata
#[derive(Debug, Default, Clone)]
pub struct Metadata {
    /// The time of the last request to synchronize contacts
    ///
    /// Used to amortize calls to the backend.
    pub contacts_sync_request_at: Option<DateTime<Utc>>,
    pub fully_migrated: Option<bool>,
}
