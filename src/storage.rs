use crate::app::{AppData, Channel, Message};
use crate::util::StatefulList;

use anyhow;
use byteorder::{BigEndian, LittleEndian};
use chrono::Utc;
use sled;
use unicode_width::UnicodeWidthStr;
use serde::{Serialize, Deserialize};
use zerocopy::{byteorder::U64, AsBytes, FromBytes, LayoutVerified, Unaligned, U16, U32};

use std::fs::File;
use std::path::Path;
use std::str;

pub trait Storage {
    fn save(data: &AppData, path: impl AsRef<Path>) -> anyhow::Result<()>;

    fn load(path: impl AsRef<Path>) -> anyhow::Result<AppData>;
}

pub struct Json;
impl Storage for Json {
    fn save(data: &AppData, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let f = File::create(path)?;
        serde_json::to_writer(f, data)?;
        Ok(())
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<AppData> {
        let f = File::open(path)?;
        let mut data: AppData = serde_json::from_reader(f)?;
        data.input_cursor = data.input.width();
        Ok(data)
    }
}

pub struct Db;

#[derive(Serialize, Deserialize, Debug)]
struct PersistedMessage {
    channel_id: u64,
    message: Message,
}

impl Db {
    fn load_channels(db: &sled::Db) -> anyhow::Result<Vec<Channel>> {
        let channels = db.open_tree(b"channels")?;

        let mut out = vec![];

        for name_value_res in &channels {
            let (_id, bytes) = name_value_res?;
            let channel: Channel = bincode::deserialize(&bytes[..]).unwrap();
            out.push(channel);
        }

        Ok(out)
    }

    fn load_messages(db: &sled::Db) -> anyhow::Result<Vec<(u64, Message)>> {
        let messages = db.open_tree(b"messages")?;

        let mut out = vec![];

        for name_value_res in &messages {
            let (_id, bytes) = name_value_res?;

            let decoded: PersistedMessage = bincode::deserialize(&bytes[..]).unwrap();
            out.push((decoded.channel_id, decoded.message));
        }

        Ok(out)
    }

    fn join(channels: Vec<Channel>, _messages: Vec<(u64, Message)>) -> anyhow::Result<AppData> {
        // TODO: join messages with channels
        Ok(AppData {
            channels: StatefulList::with_items(channels),
            input: "hello".to_owned(), // TODO: load
            input_cursor: 42,          // TODO: load
        })
    }

    fn save_channel(channels: &sled::Tree, id: u64, channel: &Channel) -> anyhow::Result<()> {
        let encoded: Vec<u8> = bincode::serialize(channel).unwrap();
        let id_value: U64<BigEndian> = U64::new(id);
        channels.insert(id_value.as_bytes(), encoded)?;
        Ok(())
    }

    /// Persists a message in a sled tree.
    /// The layout is `id -> channeld.id + message.message`.
    fn save_message(
        messages: &sled::Tree,
        id: u64,
        message: &Message,
        channel_id: u64,
    ) -> anyhow::Result<()> {
        
        let persisted = PersistedMessage {
            channel_id, message: *message
        };
        let encoded: Vec<u8> = bincode::serialize(&persisted).unwrap();
        let id_value: U64<BigEndian> = U64::new(id);
        messages.insert(id_value.as_bytes(), encoded)?;
        Ok(())
    }
}

impl Storage for Db {
    fn save(data: &AppData, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let db = sled::open(path)?;
        let messages = db.open_tree(b"messages")?;
        let channels = db.open_tree(b"channels")?;

        for (ch_count, channel) in data.channels.items.iter().enumerate() {
            Self::save_channel(&channels, ch_count as u64, channel)?;

            for (msg_count, message) in channel.messages.iter().enumerate() {
                let id: u64 = (msg_count as u64) + (ch_count as u64);
                Self::save_message(&messages, id, message, ch_count as u64)?;
            }
        }
        Ok(())
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<AppData> {
        let db = sled::open(path)?;
        let channels = Self::load_channels(&db)?;
        let messages = Self::load_messages(&db)?;
        let mut data: AppData = Self::join(channels, messages)?;
        //data.input_cursor = data.input.width();
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;
    use crate::app::Message;
    use chrono::{DateTime, NaiveDateTime, Utc};
    use std::env;

    #[test]
    fn test_save_load() {
        let channel = Channel {
            id: "BASIC".to_owned(),
            name: "BASIC".to_owned(),
            is_group: true,
            messages: vec![Message {
                from: "karsten".to_owned(),
                message: Some("hello".to_owned()),
                attachments: Vec::new(),
                arrived_at: DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(61, 0), Utc),
            }],
            unread_messages: 1,
        };
        let data = AppData {
            channels: StatefulList::with_items(vec![channel]),
            input: "hello".to_owned(),
            input_cursor: 42,
        };

        let data_path = env::current_dir()
            .expect("Could not determin current directory.")
            .join("test-db");
        Db::save(&data, &data_path).expect("Could not persist app data.");

        let loaded_data = Db::load(&data_path).expect("Could not load app data.");
        assert_eq!(loaded_data.input, "hello");
        assert_eq!(loaded_data.channels.items.len(), 1);
    }
}
