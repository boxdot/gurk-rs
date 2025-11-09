use std::collections::btree_map::Entry;
use std::{borrow::Cow, num::NonZero};
use std::{cell::Cell, collections::BTreeMap};

use get_size2::GetSize;
use lru::LruCache;
use thread_local::ThreadLocal;
use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

use super::{MessageId, Metadata, Storage};

struct LruMessageCache(Cell<LruCache<MessageId, Message>>);

impl LruMessageCache {
    fn try_get_or_insert(
        &self,
        message_id: MessageId,
        f: impl FnOnce() -> Option<Message>,
    ) -> Option<Message> {
        let empty = LruCache::unbounded();
        let mut cache = self.0.replace(empty);
        let res = cache
            .try_get_or_insert(message_id, || f().ok_or(()))
            .cloned()
            .ok();
        self.0.replace(cache);
        res
    }

    fn put(&self, message_id: MessageId, message: Message) {
        let mut cache = self.0.replace(LruCache::unbounded());
        cache.put(message_id, message);
        self.0.replace(cache);
    }
}

impl Default for LruMessageCache {
    fn default() -> Self {
        Self(Cell::new(LruCache::new(NUM_CACHED_MESSAGES)))
    }
}

type MessageIndex = BTreeMap<ChannelId, BTreeMap<usize, Option<MessageId>>>;

/// Caches the data of the underlying Storage in memory
///
/// The following data is NOT cached:
///
/// * edits
/// * count_messages
pub struct MemCache<S: Storage> {
    channels: Vec<Channel>,
    channels_index: BTreeMap<ChannelId, usize>,
    messages_cache: ThreadLocal<LruMessageCache>,
    messages_index: ThreadLocal<Cell<MessageIndex>>,
    names: BTreeMap<Uuid, String>,
    metadata: Metadata,
    storage: S,
}

impl<S: Storage> GetSize for MemCache<S> {
    fn get_heap_size(&self) -> usize {
        self.channels.get_size()
            + self.channels_index.get_size()
            + self.names.get_size()
            + self.metadata.get_size()
    }
}

const NUM_CACHED_MESSAGES: NonZero<usize> = NonZero::new(1024).unwrap();

impl<S: Storage> MemCache<S> {
    pub fn new(storage: S) -> Self {
        let mut channels: Vec<Channel> = Vec::new();
        let mut channels_index = BTreeMap::new();

        // load channels into memory
        for channel in storage.channels() {
            channels_index.insert(channel.id, channels.len());
            channels.push(channel.clone().into_owned());
        }

        let names = storage
            .names()
            .map(|(id, name)| (id, name.into_owned()))
            .collect();

        let metadata = storage.metadata().into_owned();

        Self {
            channels,
            channels_index,
            messages_cache: ThreadLocal::new(),
            messages_index: ThreadLocal::new(),
            names,
            metadata,
            storage,
        }
    }
}

impl<S: Storage> Storage for MemCache<S> {
    fn channels(&self) -> Box<dyn Iterator<Item = Cow<'_, Channel>> + '_> {
        Box::new(self.channels.iter().map(Cow::Borrowed))
    }

    fn channel(&self, channel_id: ChannelId) -> Option<Cow<'_, Channel>> {
        let idx = *self.channels_index.get(&channel_id)?;
        self.channels.get(idx).map(Cow::Borrowed)
    }

    fn store_channel(&mut self, channel: Channel) -> Cow<'_, Channel> {
        match self.channels_index.entry(channel.id) {
            Entry::Vacant(entry) => {
                entry.insert(self.channels.len());
                self.channels.push(channel.clone());
            }
            Entry::Occupied(entry) => {
                let idx = *entry.get();
                let stored_channel = &mut self.channels[idx];
                *stored_channel = channel.clone();
            }
        }
        self.storage.store_channel(channel)
    }

    fn edits(
        &self,
        message_id: MessageId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<'_, Message>> + '_> {
        self.storage.edits(message_id) // Edits are not cached
    }

    fn message(&self, message_id: MessageId) -> Option<Cow<'_, Message>> {
        let message = self
            .messages_cache
            .get_or_default()
            .try_get_or_insert(message_id, || {
                self.storage.message(message_id).map(Cow::into_owned)
            })?;
        Some(Cow::Owned(message))
    }

    fn store_message(&mut self, channel_id: ChannelId, message: Message) -> Cow<'_, Message> {
        let message_id = message.id(channel_id);
        self.messages_cache
            .get_or_default()
            .put(message_id, message.clone());
        let mut index = self.messages_index.get_or_default().take();
        index.entry(channel_id).or_default().clear();
        self.messages_index.get_or_default().set(index);
        self.storage.store_message(channel_id, message)
    }

    fn names(&self) -> Box<dyn Iterator<Item = (Uuid, Cow<'_, str>)> + '_> {
        Box::new(
            self.names
                .iter()
                .map(|(id, name)| (*id, name.as_str().into())),
        )
    }

    fn name(&self, id: Uuid) -> Option<Cow<'_, str>> {
        self.names.get(&id).map(String::as_str).map(Cow::Borrowed)
    }

    fn store_name(&mut self, id: Uuid, name: String) -> Cow<'_, str> {
        match self.names.entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(name.clone());
            }
            Entry::Occupied(mut entry) => {
                entry.insert(name.clone());
            }
        }
        self.storage.store_name(id, name)
    }

    fn metadata(&self) -> Cow<'_, Metadata> {
        Cow::Borrowed(&self.metadata)
    }

    fn store_metadata(&mut self, metadata: Metadata) -> Cow<'_, Metadata> {
        self.metadata = metadata.clone();
        self.storage.store_metadata(metadata)
    }

    fn message_channel(&self, arrived_at: u64) -> Option<ChannelId> {
        // message arrived_at to channel_id conversion is not cached
        self.storage.message_channel(arrived_at)
    }

    fn message_id_at(&self, channel_id: ChannelId, idx: usize) -> Option<MessageId> {
        let mut index = self.messages_index.get_or_default().take();
        let message_id = match index.entry(channel_id).or_default().entry(idx) {
            Entry::Vacant(entry) => {
                let message_id = self.storage.message_id_at(channel_id, idx);
                entry.insert(message_id);
                message_id
            }
            Entry::Occupied(entry) => *entry.get(),
        };
        self.messages_index.get_or_default().set(index);
        message_id
    }

    fn num_messages(&self, channel_id: ChannelId, after: u64) -> usize {
        // not cached
        self.storage.num_messages(channel_id, after)
    }
}
