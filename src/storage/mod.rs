mod copy;
mod forgetful;
mod sql;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

pub use copy::sync_from_signal;
pub use forgetful::ForgetfulStorage;
pub use sql::SqliteStorage;

/// Storage of channels, messages, names and metadata.
///
/// Used to persist the data to disk.
pub trait Storage {
    /// Returns the list of channels
    ///
    /// The order is from most recent to least recent, that is, descending by arrived_at.
    fn channels(&self) -> Vec<Channel>;

    /// Gets the channel by id
    fn channel(&self, channel_id: ChannelId) -> Option<Channel>;

    /// Stores the given `channel`
    fn store_channel(&mut self, channel: &Channel);

    /// The newest `limit` messages, ascending by arrived_at.
    fn messages_tail(&self, channel_id: ChannelId, limit: usize) -> Vec<Message>;

    /// Up to `limit` messages strictly older than `anchor`, ascending by arrived_at.
    fn messages_before(&self, channel_id: ChannelId, anchor: u64, limit: usize) -> Vec<Message>;

    /// Up to `limit` messages strictly newer than `anchor`, ascending by arrived_at.
    fn messages_after(&self, channel_id: ChannelId, anchor: u64, limit: usize) -> Vec<Message>;

    // Messages window functions

    /// Messages sorted by arrived_at in ascending order
    ///
    /// No edited messages must be included.
    fn messages(&self, channel_id: ChannelId) -> Box<dyn DoubleEndedIterator<Item = Message> + '_>;
    /// Gets the message by id
    fn message(&self, message_id: MessageId) -> Option<Message>;

    fn edits(&self, message_id: MessageId) -> Box<dyn DoubleEndedIterator<Item = Message> + '_>;

    fn last_message(&self, channel_id: ChannelId) -> Option<Message> {
        self.messages_tail(channel_id, 1).pop()
    }

    fn messages_count_after(&self, channel_id: ChannelId, arrived_at: u64) -> usize;

    fn remove_expired(&self, now_ms: u64) -> Vec<MessageId>;

    fn next_expiring_at(&self) -> Option<u64>;

    /// Stores the message for the given `channel_id`.
    ///
    /// The channel with the given `channel_id` must already exist.
    fn store_message(&mut self, channel_id: ChannelId, message: &Message);

    /// Applies an edit and returns the updated original message, or `None` if the message pointed
    /// to by `target_sent_timestamp` does not exist.
    fn store_edited_message(
        &mut self,
        channel_id: ChannelId,
        target_sent_timestampt: u64,
        message: Message,
    ) -> Option<Message> {
        // Note: target_sent_timestamp points to the previous edit or the original message
        let prev_edited = self.message(MessageId::new(channel_id, target_sent_timestampt))?;

        // get original message
        let mut original = if let Some(arrived_at) = prev_edited.edit {
            // previous edit => get original message
            self.message(MessageId::new(channel_id, arrived_at))?
        } else {
            // original message => first edit
            let original = prev_edited;

            // preserve body of the original message; it is replaced below
            let mut preserved = original.clone();
            preserved.arrived_at = original.arrived_at + 1;
            preserved.edit = Some(original.arrived_at);
            self.store_message(channel_id, &preserved);

            original
        };

        // store the incoming edit
        let body = message.message.clone();
        self.store_message(
            channel_id,
            &Message {
                edit: Some(original.arrived_at),
                ..message
            },
        );

        // override the body of the original message
        original.message = body;
        original.edited = true;
        self.store_message(channel_id, &original);
        Some(original)
    }

    /// Marks a message as deleted (remote delete / delete for everyone).
    ///
    /// Clears the message body and attachments, and sets the `deleted` flag.
    /// Returns `true` if the message existed.
    fn delete_message(&mut self, message_id: MessageId) -> bool {
        let Some(mut message) = self.message(message_id) else {
            return false;
        };
        message.message = None;
        message.attachments.clear();
        message.body_ranges.clear();
        message.deleted = true;
        self.store_message(message_id.channel_id, &message);
        true
    }

    /// Fully removes a message from storage (delete for me)
    fn remove_message(&mut self, message_id: MessageId);

    /// Names of contacts
    fn names(&self) -> Box<dyn Iterator<Item = (Uuid, String)> + '_>;
    /// Gets the name for the given contact `id`
    fn name(&self, id: Uuid) -> Option<String>;
    /// Stores a name for the given contact `id`
    ///
    /// If the name with this `id` already exists in the storage, it is overridden. Otherwise, it
    /// the name is added to the storage.
    fn store_name(&mut self, id: Uuid, name: &str);

    /// Returns the metadata containing persisted flags and settings
    fn metadata(&self) -> Metadata;
    /// Stores the new metadata in the storage overriding the previous one
    fn store_metadata(&mut self, metadata: &Metadata);

    /// Persists the data in the storage
    ///
    /// ## Implementation note
    ///
    /// The implementers of this trait, can persist for each store call, if it is efficient enough.
    /// This methods must guarantee that the data is persisted in any case.
    fn save(&mut self);

    // /// Returns `true` if this storage does not contains any channels and no names
    // fn is_empty(&self) -> bool {
    //     self.channels().next().is_none() && self.names().next().is_none()
    // }
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
