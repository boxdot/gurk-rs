use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use uuid::Uuid;

use crate::data::{Channel, ChannelId, GroupData, Message, TypingSet};

use super::{MessageId, Metadata, Storage};

pub struct JsonStorage {
    data_path: PathBuf,
    data: JsonStorageData,
    is_dirty: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct JsonStorageData {
    channels: JsonChannels,
    /// Names retrieved from:
    /// - profiles, when registered as main device)
    /// - contacts, when linked as secondary device
    /// - UUID when both have failed
    ///
    /// Do not use directly, use [`App::name_by_id`] instead.
    names: HashMap<Uuid, String>,
    #[serde(default)]
    contacts_sync_request_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct JsonChannels {
    items: Vec<JsonChannel>,
}

/// Proxy type which allows us to apply post-deserialization conversion.
///
/// Used to migrate the schema. Change this type only in backwards-compatible way.
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonChannel {
    id: ChannelId,
    name: String,
    #[serde(default)]
    group_data: Option<GroupData>,
    messages: Vec<Message>,
    #[serde(default)]
    unread_messages: u32,
    #[serde(skip)]
    typing: Option<TypingSet>,
}

impl From<&JsonChannel> for Channel {
    fn from(channel: &JsonChannel) -> Self {
        let is_group = channel.group_data.is_some();
        Self {
            id: channel.id,
            name: channel.name.clone(),
            group_data: channel.group_data.clone(),
            unread_messages: channel.unread_messages,
            typing: channel
                .typing
                .clone()
                .unwrap_or_else(|| TypingSet::new(is_group)),
        }
    }
}

impl From<(Channel, Vec<Message>)> for JsonChannel {
    fn from((channel, messages): (Channel, Vec<Message>)) -> Self {
        Self {
            id: channel.id,
            name: channel.name,
            group_data: channel.group_data,
            messages,
            unread_messages: channel.unread_messages,
            typing: Some(channel.typing),
        }
    }
}

impl JsonStorage {
    pub fn new(data_path: &Path, fallback_data_path: Option<&Path>) -> anyhow::Result<Self> {
        let mut data_path = data_path;
        if !data_path.exists() {
            // try also to load from a fallback (legacy) data path
            if let Some(fallback_data_path) = fallback_data_path.as_ref() {
                data_path = fallback_data_path;
            }
        }

        // if data file exists, be conservative and fail rather than overriding and losing the messages
        let mut data = if data_path.exists() {
            Self::load_data_from(data_path).with_context(|| {
                format!(
                    "failed to load stored data from '{}':\n\
            This might happen due to incompatible data model when Gurk is upgraded.\n\
            Please consider to backup your messages and then remove the store.",
                    data_path.display()
                )
            })?
        } else {
            Self::load_data_from(data_path).unwrap_or_default()
        };

        for channel in &mut data.channels.items {
            // migration:
            // The master key in ChannelId::Group was replaced by group identifier,
            // the former was stored in group_data.
            match (channel.id, channel.group_data.as_mut()) {
                (ChannelId::Group(id), Some(group_data))
                    if group_data.master_key_bytes == [0; 32] =>
                {
                    group_data.master_key_bytes = id;
                    channel.id = ChannelId::from_master_key_bytes(id)?;
                }
                _ => (),
            }
            // invariant: messages are sorted by arrived_at
            channel.messages.sort_unstable_by_key(|msg| msg.arrived_at);
        }

        Ok(Self {
            data_path: data_path.into(),
            data,
            is_dirty: false,
        })
    }

    fn load_data_from(data_path: &Path) -> anyhow::Result<JsonStorageData> {
        info!("loading app data from: {}", data_path.display());
        let f = BufReader::new(File::open(data_path)?);
        Ok(serde_json::from_reader(f)?)
    }

    fn try_save(&mut self) -> anyhow::Result<()> {
        if self.is_dirty {
            info!("saving app data to: {}", self.data_path.display());
            let f = BufWriter::new(File::create(&self.data_path)?);
            serde_json::to_writer(f, &self.data)?;
            self.is_dirty = false;
        }
        Ok(())
    }
}

impl Storage for JsonStorage {
    fn channels<'s>(&'s self) -> Box<dyn Iterator<Item = Cow<Channel>> + 's> {
        Box::new(
            self.data
                .channels
                .items
                .iter()
                .map(Into::into)
                .map(Cow::Owned),
        )
    }

    fn channel(&self, channel_id: ChannelId) -> Option<Cow<Channel>> {
        self.data
            .channels
            .items
            .iter()
            .find(|channel| channel.id == channel_id)
            .map(Into::into)
            .map(Cow::Owned)
    }

    fn store_channel(&mut self, channel: Channel) -> Cow<Channel> {
        let channel_idx = if let Some(idx) = self
            .data
            .channels
            .items
            .iter_mut()
            .position(|ch| ch.id == channel.id)
        {
            let stored_channel = &mut self.data.channels.items[idx];
            let messages = std::mem::take(&mut stored_channel.messages);
            *stored_channel = (channel, messages).into();
            idx
        } else {
            let channel = (channel, Vec::new()).into();
            let idx = self.data.channels.items.len();
            self.data.channels.items.push(channel);
            idx
        };
        self.is_dirty = true;
        Cow::Owned(Channel::from(&self.data.channels.items[channel_idx]))
    }

    fn messages<'s>(
        &'s self,
        channel_id: ChannelId,
    ) -> Box<dyn DoubleEndedIterator<Item = Cow<Message>> + 's> {
        if let Some(channel) = self
            .data
            .channels
            .items
            .iter()
            .find(|ch| ch.id == channel_id)
        {
            Box::new(channel.messages.iter().map(Cow::Borrowed))
        } else {
            Box::new(std::iter::empty())
        }
    }

    fn message(&self, message_id: MessageId) -> Option<Cow<Message>> {
        let channel = self
            .data
            .channels
            .items
            .iter()
            .find(|ch| ch.id == message_id.channel_id)?;
        let message = channel
            .messages
            .iter()
            .find(|message| message.arrived_at == message_id.arrived_at)?;
        Some(Cow::Borrowed(message))
    }

    fn store_message(&mut self, channel_id: ChannelId, message: Message) -> Cow<Message> {
        let channel_idx = self
            .data
            .channels
            .items
            .iter()
            .position(|ch| ch.id == channel_id)
            .expect("no such channel");
        let idx = {
            let channel = &mut self.data.channels.items[channel_idx];
            match channel
                .messages
                .binary_search_by_key(&message.arrived_at, |msg| msg.arrived_at)
            {
                Ok(idx) => {
                    let stored_message = &mut channel.messages[idx];
                    *stored_message = message;
                    idx
                }
                Err(idx) => {
                    channel.messages.insert(idx, message);
                    idx
                }
            }
        };
        self.is_dirty = true;
        Cow::Borrowed(&self.data.channels.items[channel_idx].messages[idx])
    }

    fn names<'s>(&'s self) -> Box<dyn Iterator<Item = (Uuid, Cow<str>)> + 's> {
        Box::new(
            self.data
                .names
                .iter()
                .map(|(id, name)| (*id, name.as_str().into())),
        )
    }

    fn name(&self, id: Uuid) -> Option<Cow<str>> {
        self.data
            .names
            .get(&id)
            .map(String::as_str)
            .map(Cow::Borrowed)
    }

    fn store_name(&mut self, id: Uuid, name: String) -> Cow<str> {
        match self.data.names.entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(name);
            }
            Entry::Occupied(mut entry) => {
                entry.insert(name);
            }
        }
        self.is_dirty = true;
        Cow::Borrowed(&self.data.names[&id])
    }

    fn metadata(&self) -> Cow<Metadata> {
        Cow::Owned(Metadata {
            contacts_sync_request_at: self.data.contacts_sync_request_at,
            fully_migrated: None,
        })
    }

    fn store_metadata(&mut self, metadata: Metadata) -> Cow<Metadata> {
        let Metadata {
            contacts_sync_request_at,
            fully_migrated: _unsupported_in_json,
        } = metadata;
        self.data.contacts_sync_request_at = contacts_sync_request_at;
        self.is_dirty = true;
        Cow::Owned(metadata)
    }

    fn save(&mut self) {
        if let Err(e) = self.try_save() {
            error!(error =% e, "failed to save json storage");
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use tempfile::NamedTempFile;
    use uuid::Uuid;

    use crate::data::TypingSet;

    use super::*;

    #[test]
    fn test_json_storage_data_model() {
        let user_id1: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();
        let user_id2: Uuid = "a955d20f-6b83-4e69-846e-a99b1779ff7a".parse().unwrap();
        let user_id3: Uuid = "ac9b8aa1-691a-47e1-a566-d3e942945d07".parse().unwrap();
        let channel1 = JsonChannel {
            id: user_id1.into(),
            name: "direct-channel".to_string(),
            group_data: None,
            messages: vec![Message {
                from_id: user_id2,
                message: Some("hello".into()),
                arrived_at: 1664832050000,
                quote: Default::default(),
                attachments: Default::default(),
                reactions: Default::default(),
                receipt: Default::default(),
                body_ranges: Default::default(),
                send_failed: Default::default(),
                edit: Default::default(),
                edited: Default::default(),
            }],
            unread_messages: 1,
            typing: Some(TypingSet::SingleTyping(false)),
        };
        let channel2 = JsonChannel {
            id: ChannelId::Group(*b"4149b9686807fdb4a8c95d9b5413bbcd"),
            name: "group-channel".to_string(),
            group_data: None,
            messages: vec![Message {
                from_id: user_id3,
                message: Some("world".into()),
                arrived_at: 1664832050001,
                quote: Default::default(),
                attachments: Default::default(),
                reactions: Default::default(),
                receipt: Default::default(),
                body_ranges: Default::default(),
                send_failed: Default::default(),
                edit: Default::default(),
                edited: Default::default(),
            }],
            unread_messages: 2,
            typing: Some(TypingSet::GroupTyping(Default::default())),
        };
        let names = [
            (user_id1, "ellie".to_string()),
            (user_id2, "joel".to_string()),
        ];
        let data = JsonStorageData {
            channels: JsonChannels {
                items: vec![channel1, channel2],
            },
            names: names.into_iter().collect(),
            contacts_sync_request_at: DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z")
                .ok()
                .map(|dt| dt.with_timezone(&Utc)),
        };

        let mut settings = insta::Settings::clone_current();
        settings.set_sort_maps(true);
        settings.bind(|| {
            insta::assert_json_snapshot!(data);
        });
    }

    fn json_storage_from_snapshot() -> impl Storage {
        let json =
            include_str!("snapshots/gurk__storage__json__tests__json_storage_data_model.snap")
                .rsplit("---")
                .next()
                .unwrap();
        let f = NamedTempFile::new().unwrap();
        std::fs::write(&f, json.as_bytes()).unwrap();

        JsonStorage::new(f.path(), None).unwrap()
    }

    #[test]
    fn test_json_storage_channels() {
        let storage = json_storage_from_snapshot();
        let channels: Vec<_> = storage.channels().collect();
        assert_eq!(channels.len(), 2);
        assert_eq!(storage.channel(channels[0].id).unwrap().id, channels[0].id);
        assert_eq!(storage.channel(channels[1].id).unwrap().id, channels[1].id);
    }

    #[test]
    fn test_json_storage_store_existing_channel() {
        let mut storage = json_storage_from_snapshot();
        let id: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();
        let mut channel = storage.channel(id.into()).unwrap().into_owned();
        channel.name = "new name".to_string();
        channel.unread_messages = 23;

        let stored_channel = storage.store_channel(channel);
        assert_eq!(stored_channel.id, id);
        assert_eq!(stored_channel.name, "new name");
        assert_eq!(stored_channel.unread_messages, 23);

        let channels: Vec<_> = storage.channels().collect();
        assert_eq!(channels.len(), 2);
        assert_eq!(storage.channel(channels[0].id).unwrap().id, channels[0].id);
        assert_eq!(storage.channel(channels[1].id).unwrap().id, channels[1].id);

        let channel = storage.channel(channels[0].id).unwrap();
        assert_eq!(channel.id, id);
        assert_eq!(channel.name, "new name");
        assert_eq!(channel.unread_messages, 23);
    }

    #[test]
    fn test_json_storage_store_new_channel() {
        let mut storage = json_storage_from_snapshot();
        let id: Uuid = "e3690a5f-70a4-4a49-8125-ca689adb2d9e".parse().unwrap();
        storage.store_channel(Channel {
            id: id.into(),
            name: "test".to_string(),
            group_data: None,
            unread_messages: 42,
            typing: TypingSet::SingleTyping(false),
        });
        let channels: Vec<_> = storage.channels().collect();
        assert_eq!(channels.len(), 3);
        assert_eq!(storage.channel(channels[0].id).unwrap().id, channels[0].id);
        assert_eq!(storage.channel(channels[1].id).unwrap().id, channels[1].id);
        let channel = storage.channel(channels[2].id).unwrap();
        assert_eq!(channel.id, id);
        assert_eq!(channel.name, "test");
        assert_eq!(channel.unread_messages, 42);
    }

    #[test]
    fn test_json_storage_messages() {
        let storage = json_storage_from_snapshot();
        let id: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();

        let messages: Vec<_> = storage.messages(id.into()).collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message.as_deref(), Some("hello"));

        let arrived_at = messages[0].arrived_at;
        let message = storage
            .message(MessageId::new(id.into(), arrived_at))
            .unwrap();
        assert_eq!(message.arrived_at, arrived_at);
        assert_eq!(message.message.as_deref(), Some("hello"));
    }

    #[test]
    fn test_json_storage_store_existing_message() {
        let mut storage = json_storage_from_snapshot();
        let id: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();
        let arrived_at = 1664832050000;
        let mut message = storage
            .message(MessageId::new(id.into(), arrived_at))
            .unwrap()
            .into_owned();
        message.message = Some("changed".to_string());

        let arrived_at = message.arrived_at;
        let stored_message = storage.store_message(id.into(), message);
        assert_eq!(stored_message.arrived_at, arrived_at);
        assert_eq!(stored_message.message.as_deref(), Some("changed"));

        let messages: Vec<_> = storage.messages(id.into()).collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].arrived_at, arrived_at);
        assert_eq!(messages[0].message.as_deref(), Some("changed"));
    }

    #[test]
    fn test_json_storage_store_new_message() {
        let mut storage = json_storage_from_snapshot();
        let id: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();
        let arrived_at = 1664832050001;
        assert_eq!(storage.message(MessageId::new(id.into(), arrived_at)), None);

        let stored_message = storage.store_message(
            id.into(),
            Message {
                from_id: id,
                message: Some("new msg".to_string()),
                arrived_at,
                quote: Default::default(),
                attachments: Default::default(),
                reactions: Default::default(),
                receipt: Default::default(),
                body_ranges: Default::default(),
                send_failed: Default::default(),
                edit: Default::default(),
                edited: Default::default(),
            },
        );

        assert_eq!(stored_message.arrived_at, arrived_at);
        assert_eq!(stored_message.message.as_deref(), Some("new msg"));

        let messages: Vec<_> = storage.messages(id.into()).collect();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].arrived_at, arrived_at);
        assert_eq!(messages[1].message.as_deref(), Some("new msg"));
    }

    #[test]
    fn test_json_storage_names() {
        let mut storage = json_storage_from_snapshot();
        let id1: Uuid = "966960e0-a8cd-43f1-ac7a-2c986dd470cd".parse().unwrap();
        let id2: Uuid = "a955d20f-6b83-4e69-846e-a99b1779ff7a".parse().unwrap();
        let id3: Uuid = "91a6315b-027c-44ce-bacb-4d5cf012ba8c".parse().unwrap();

        assert_eq!(storage.names().count(), 2);
        assert_eq!(storage.name(id1).unwrap(), "ellie");
        assert_eq!(storage.name(id2).unwrap(), "joel");

        assert_eq!(storage.store_name(id3, "abby".to_string()), "abby");
        assert_eq!(storage.names().count(), 3);
        assert_eq!(storage.name(id1).unwrap(), "ellie");
        assert_eq!(storage.name(id2).unwrap(), "joel");
        assert_eq!(storage.name(id3).unwrap(), "abby");
    }

    #[test]
    fn test_json_storage_metadata() {
        let mut storage = json_storage_from_snapshot();
        let dt = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        assert_eq!(storage.metadata().contacts_sync_request_at, Some(dt));
        let dt = Utc::now();
        assert_eq!(
            storage
                .store_metadata(Metadata {
                    contacts_sync_request_at: Some(dt),
                    fully_migrated: None,
                })
                .contacts_sync_request_at,
            Some(dt)
        );
        assert_eq!(storage.metadata().contacts_sync_request_at, Some(dt));
    }
}
