use crate::app::{AppData, Channel, Message};

use anyhow;
use byteorder::{BigEndian, LittleEndian};
use chrono::Utc;
use sled;
use unicode_width::UnicodeWidthStr;
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

impl Db {
    fn load_channels(db: &sled::Db) -> anyhow::Result<Vec<Channel>> {
        unimplemented!()
    }

    fn load_messages(db: &sled::Db) -> anyhow::Result<Vec<Message>> {
        let messages = db.open_tree(b"messages")?;

        let mut out = vec![];

        for name_value_res in &messages {
            let (_id, bytes) = name_value_res?;
            let content = str::from_utf8(&bytes)?;
            let message = Message {
                from: "not saved".to_owned(),
                message: Some(content.to_owned()),
                attachments: Vec::new(),
                arrived_at: Utc::now(),
            };
            out.push(message);
        }

        Ok(out)
    }

    fn join(channels: Vec<Channel>, messages: Vec<Message>) -> anyhow::Result<AppData> {
        unimplemented!()
    }

    fn save_channel(channels: &sled::Tree, id: u64, channel: &Channel) -> anyhow::Result<()> {
        let mut channel_value = vec![];
        channel_value.extend_from_slice(channel.name.as_bytes());
        let id_value: U64<BigEndian> = U64::new(id);

        channels.insert(id_value.as_bytes(), channel_value)?;
        Ok(())
    }

    fn save_message(messages: &sled::Tree, id: u64, message: &Message) -> anyhow::Result<()> {
        let content = match message.message {
            Some(ref s) => s.as_bytes(),
            None => b"",
        };

        let mut message_value = vec![];
        message_value.extend_from_slice(content);
        let id_value: U64<BigEndian> = U64::new(id);
        messages.insert(id_value.as_bytes(), message_value)?;
        Ok(())
    }
}

impl Storage for Db {
    fn save(data: &AppData, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let db = sled::open(path)?;
        let messages = db.open_tree(b"messages")?;
        let channels = db.open_tree(b"messages")?;

        for (ch_count, channel) in data.channels.items.iter().enumerate() {
            Self::save_channel(&channels, ch_count as u64, channel)?;

            for (msg_count, message) in channel.messages.iter().enumerate() {
                let id: u64 = (msg_count as u64) + (ch_count as u64);
                Self::save_message(&messages, id, message)?;
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
