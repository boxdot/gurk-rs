use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use uuid::Uuid;

use crate::data::{Channel, ChannelId, Message};

use super::{MessageId, Metadata, Storage};

/// Caches the data of the underlying Storage in memory
///
/// The following data is NOT cached:
///
/// * edits
pub struct MemCache<S: Storage> {
    channels: Vec<Channel>,
    channels_index: BTreeMap<ChannelId, usize>,
    messages: BTreeMap<ChannelId, Vec<Message>>,
    messages_index: BTreeMap<MessageId, usize>,
    names: BTreeMap<Uuid, String>,
    metadata: Metadata,
    storage: S,
}

impl<S: Storage> MemCache<S> {
    pub fn new(storage: S) -> Self {
        let mut channels: Vec<Channel> = Vec::new();
        let mut channels_index = BTreeMap::new();
        let mut messages: BTreeMap<ChannelId, Vec<Message>> = BTreeMap::new();
        let mut messages_index: BTreeMap<MessageId, usize> = BTreeMap::new();

        // build in-memory cache
        for channel in storage.channels() {
            let channel_messages = messages.entry(channel.id).or_default();
            for message in storage.messages(channel.id) {
                let message_id = MessageId::new(channel.id, message.arrived_at);
                messages_index.insert(message_id, channel_messages.len());
                channel_messages.push(message.clone().into_owned());
            }
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
            messages,
            messages_index,
            names,
            metadata,
            storage,
        }
    }
}

impl<S: Storage> Storage for MemCache<S> {
    fn channels(&self) -> Box<dyn Iterator<Item = Cow<Channel>> + '_> {
        Box::new(self.channels.iter().map(Cow::Borrowed))
    }

    fn channel(&self, channel_id: ChannelId) -> Option<Cow<Channel>> {
        let idx = *self.channels_index.get(&channel_id)?;
        self.channels.get(idx).map(Cow::Borrowed)
    }

    fn store_channel(&mut self, channel: Channel) -> Cow<Channel> {
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

    fn messages(
        &self,
        channel_id: ChannelId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + '_> {
        if let Some(messages) = self.messages.get(&channel_id) {
            Box::new(messages.iter().map(Cow::Borrowed))
        } else {
            Box::new(std::iter::empty())
        }
    }

    fn edits(
        &self,
        message_id: MessageId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + '_> {
        self.storage.edits(message_id) // Edits are not cached
    }

    fn message(&self, message_id: MessageId) -> Option<Cow<Message>> {
        let messages = self.messages.get(&message_id.channel_id)?;
        let cached = self
            .messages_index
            .get(&message_id)
            .and_then(|&idx| messages.get(idx).map(Cow::Borrowed));
        if let Some(message) = cached {
            Some(message)
        } else {
            let message = self.storage.message(message_id)?;
            Some(message)
        }
    }

    fn store_message(&mut self, channel_id: ChannelId, message: Message) -> Cow<Message> {
        let message_id = MessageId::new(channel_id, message.arrived_at);
        match self.messages_index.entry(message_id) {
            Entry::Vacant(entry) => {
                let messages = self.messages.entry(channel_id).or_default();
                entry.insert(messages.len());
                messages.push(message.clone());
            }
            Entry::Occupied(entry) => {
                let idx = *entry.get();
                let messages = self.messages.entry(channel_id).or_default();
                let stored_message = &mut messages[idx];
                *stored_message = message.clone();
            }
        }
        self.storage.store_message(channel_id, message)
    }

    fn names(&self) -> Box<dyn Iterator<Item = (Uuid, Cow<str>)> + '_> {
        Box::new(
            self.names
                .iter()
                .map(|(id, name)| (*id, name.as_str().into())),
        )
    }

    fn name(&self, id: Uuid) -> Option<Cow<str>> {
        self.names.get(&id).map(String::as_str).map(Cow::Borrowed)
    }

    fn store_name(&mut self, id: Uuid, name: String) -> Cow<str> {
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

    fn metadata(&self) -> Cow<Metadata> {
        Cow::Borrowed(&self.metadata)
    }

    fn store_metadata(&mut self, metadata: Metadata) -> Cow<Metadata> {
        self.metadata = metadata.clone();
        self.storage.store_metadata(metadata)
    }

    fn save(&mut self) {
        self.storage.save();
    }

    fn message_channel(&self, arrived_at: u64) -> Option<ChannelId> {
        // message arrived_at to channel_id conversion is not cached
        self.storage.message_channel(arrived_at)
    }
}
