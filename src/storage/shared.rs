//! Shared storage wrapper for concurrent access
//!
//! Wraps a SqliteStorage in Arc<tokio::sync::RwLock> for safe concurrent access
//! from both the main App and the API server.

use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

use super::sql::SqliteStorage;
use super::{MessageId, Metadata, Storage};

/// A thread-safe shared storage wrapper.
///
/// Wraps SqliteStorage in an Arc<tokio::sync::RwLock> to allow both the App
/// (which uses Box<dyn Storage> with blocking access) and the API server
/// (which uses Arc<RwLock<S>> with async access) to share the same underlying
/// storage instance.
pub struct SharedStorage {
    inner: Arc<RwLock<SqliteStorage>>,
}

impl SharedStorage {
    /// Creates a new SharedStorage wrapping the given SqliteStorage.
    pub fn new(storage: SqliteStorage) -> Self {
        Self {
            inner: Arc::new(RwLock::new(storage)),
        }
    }

    /// Returns a clone of the inner Arc for sharing with the API.
    pub fn inner(&self) -> Arc<RwLock<SqliteStorage>> {
        Arc::clone(&self.inner)
    }
}

impl Storage for SharedStorage {
    fn channels(&self) -> Vec<Channel> {
        self.inner.blocking_read().channels()
    }

    fn channel(&self, channel_id: ChannelId) -> Option<Channel> {
        self.inner.blocking_read().channel(channel_id)
    }

    fn store_channel(&mut self, channel: &Channel) {
        self.inner.blocking_write().store_channel(channel)
    }

    fn messages_tail(&self, channel_id: ChannelId, limit: usize) -> Vec<Message> {
        self.inner.blocking_read().messages_tail(channel_id, limit)
    }

    fn messages_before(&self, channel_id: ChannelId, anchor: u64, limit: usize) -> Vec<Message> {
        self.inner
            .blocking_read()
            .messages_before(channel_id, anchor, limit)
    }

    fn messages_after(&self, channel_id: ChannelId, anchor: u64, limit: usize) -> Vec<Message> {
        self.inner
            .blocking_read()
            .messages_after(channel_id, anchor, limit)
    }

    fn messages(&self, channel_id: ChannelId) -> Box<dyn DoubleEndedIterator<Item = Message> + '_> {
        // Collect while holding the lock to avoid lifetime issues
        let msgs: Vec<_> = self
            .inner
            .blocking_read()
            .messages(channel_id)
            .collect();
        Box::new(msgs.into_iter())
    }

    fn message(&self, message_id: MessageId) -> Option<Message> {
        self.inner.blocking_read().message(message_id)
    }

    fn edits(&self, message_id: MessageId) -> Box<dyn DoubleEndedIterator<Item = Message> + '_> {
        // Collect while holding the lock to avoid lifetime issues
        let msgs: Vec<_> = self.inner.blocking_read().edits(message_id).collect();
        Box::new(msgs.into_iter())
    }

    fn messages_count_after(&self, channel_id: ChannelId, arrived_at: u64) -> usize {
        self.inner
            .blocking_read()
            .messages_count_after(channel_id, arrived_at)
    }

    fn remove_expired(&self, now_ms: u64) -> Vec<MessageId> {
        // Note: This calls &self method on SqliteStorage but needs write access
        // because it modifies the database
        self.inner.blocking_write().remove_expired(now_ms)
    }

    fn next_expiring_at(&self) -> Option<u64> {
        self.inner.blocking_read().next_expiring_at()
    }

    fn store_message(&mut self, channel_id: ChannelId, message: &Message) {
        self.inner
            .blocking_write()
            .store_message(channel_id, message)
    }

    fn remove_message(&mut self, message_id: MessageId) {
        self.inner.blocking_write().remove_message(message_id)
    }

    fn names(&self) -> Box<dyn Iterator<Item = (Uuid, String)> + '_> {
        // Collect while holding the lock to avoid lifetime issues
        let names: Vec<_> = self.inner.blocking_read().names().collect();
        Box::new(names.into_iter())
    }

    fn name(&self, id: Uuid) -> Option<String> {
        self.inner.blocking_read().name(id)
    }

    fn store_name(&mut self, id: Uuid, name: &str) {
        self.inner.blocking_write().store_name(id, name)
    }

    fn metadata(&self) -> Metadata {
        self.inner.blocking_read().metadata()
    }

    fn store_metadata(&mut self, metadata: &Metadata) {
        self.inner.blocking_write().store_metadata(metadata)
    }

    fn save(&mut self) {
        self.inner.blocking_write().save()
    }
}
