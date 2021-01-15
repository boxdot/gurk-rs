use crate::app::{AppData, Channel};
use crate::util::StatefulList;

use anyhow;
use byteorder::BigEndian;
use sled;
use unicode_width::UnicodeWidthStr;
use zerocopy::{byteorder::U64, AsBytes};

use std::fs::File;
use std::path::Path;

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

    fn save_channel(channels: &sled::Tree, id: u64, channel: &Channel) -> anyhow::Result<()> {
        let encoded: Vec<u8> = bincode::serialize(channel).unwrap();
        let id_value: U64<BigEndian> = U64::new(id);
        channels.insert(id_value.as_bytes(), encoded)?;
        Ok(())
    }
}

impl Storage for Db {
    fn save(data: &AppData, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let db = sled::open(path)?;
        let channels = db.open_tree(b"channels")?;

        for (ch_count, channel) in data.channels.items.iter().enumerate() {
            Self::save_channel(&channels, ch_count as u64, channel)?;
        }
        Ok(())
    }

    fn load(path: impl AsRef<Path>) -> anyhow::Result<AppData> {
        let db = sled::open(path)?;
        let channels = Self::load_channels(&db)?;
        let data: AppData = AppData {
            channels: StatefulList::with_items(channels),
            input: "what should go here?".to_owned(),
            input_cursor: 0,
        };
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
        let channel = loaded_data.channels.items.first().unwrap();
        assert_eq!(channel.id, "BASIC");
        assert_eq!(channel.messages.len(), 1);
    }
}
