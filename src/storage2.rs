use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::data::{Channel, ChannelId, JsonChannel, Message};

pub trait Storage {
    fn channels(&self) -> Box<dyn Iterator<Item = Cow<Channel>>>;
    fn channel(&self, channel_id: ChannelId) -> Option<Cow<Channel>>;
    fn store_channel(&mut self, channel: Channel) -> Option<Cow<Channel>>;

    fn messages(&self) -> Box<dyn Iterator<Item = Cow<Message>>>;
    fn message(&self) -> Option<Cow<Message>>;
    fn store_message(&mut self, channel_id: ChannelId, message: Message) -> Option<Cow<Message>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MessageId {
    channel_id: ChannelId,
    arrived_at: u64,
}

impl MessageId {
    fn new(channel_id: ChannelId, arrived_at: u64) -> Self {
        Self {
            channel_id,
            arrived_at,
        }
    }
}

pub struct JsonStorage {
    channels: Vec<Channel>,
    channels_index: BTreeMap<ChannelId, usize>,
    messages: BTreeMap<ChannelId, Vec<Message>>,
    messages_index: BTreeMap<MessageId, usize>,
    data_path: PathBuf,
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
        let data = if data_path.exists() {
            Self::load_data_from(&data_path).with_context(|| {
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

        dbg!(&data);

        let mut channels: Vec<Channel> = Vec::with_capacity(data.channels.items.len());
        let mut channels_index = BTreeMap::new();
        let mut messages: BTreeMap<ChannelId, Vec<Message>> = BTreeMap::new();
        let mut messages_index: BTreeMap<MessageId, usize> = BTreeMap::new();

        for mut channel in data
            .channels
            .items
            .into_iter()
            .map(Channel::try_from)
            .filter_map(Result::ok)
        {
            let channel_messages = messages.entry(channel.id).or_default();
            for message in std::mem::take(&mut channel.messages).items {
                let message_id = MessageId::new(channel.id, message.arrived_at);
                messages_index.insert(message_id, channel_messages.len());
                channel_messages.push(message);
            }
            channels_index.insert(channel.id, channels.len());
            channels.push(channel);
        }

        Ok(Self {
            channels,
            channels_index,
            messages,
            messages_index,
            data_path: data_path.into(),
        })
    }

    fn load_data_from(data_path: &Path) -> anyhow::Result<JsonStorageData> {
        info!("loading app data from: {}", data_path.display());
        let f = BufReader::new(File::open(data_path)?);
        dbg!(&f);
        Ok(serde_json::from_reader(f)?)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct JsonStorageData {
    channels: JsonChannels,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct JsonChannels {
    items: Vec<JsonChannel>,
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;
    use uuid::Uuid;

    use crate::util::StatefulList;

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
            messages: StatefulList::with_items(vec![Message {
                from_id: user_id2,
                message: Some("hello".into()),
                arrived_at: 1664832050000,
                quote: Default::default(),
                attachments: Default::default(),
                reactions: Default::default(),
                receipt: Default::default(),
            }]),
            unread_messages: 1,
        };
        let channel2 = JsonChannel {
            id: ChannelId::Group(*b"4149b9686807fdb4a8c95d9b5413bbcd"),
            name: "group-channel".to_string(),
            group_data: None,
            messages: StatefulList::with_items(vec![Message {
                from_id: user_id3,
                message: Some("world".into()),
                arrived_at: 1664832050001,
                quote: Default::default(),
                attachments: Default::default(),
                reactions: Default::default(),
                receipt: Default::default(),
            }]),
            unread_messages: 2,
        };
        let data = JsonStorageData {
            channels: JsonChannels {
                items: vec![channel1, channel2],
            },
        };
        insta::assert_json_snapshot!(data);
    }

    #[test]
    fn test_json_storage_new() {
        let json = include_str!("snapshots/gurk__storage2__tests__json_storage_data_model.snap")
            .rsplit("---")
            .next()
            .unwrap();
        print!("{json}");
        let f = NamedTempFile::new().unwrap();
        std::fs::write(&f, json.as_bytes()).unwrap();

        let storage = JsonStorage::new(f.path(), None).unwrap();
        assert_eq!(storage.channels.len(), 2);
        assert_eq!(storage.channels.len(), storage.channels_index.len());
        assert_eq!(storage.channels.len(), storage.messages.len());
    }
}
