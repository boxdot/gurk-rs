use crate::app::{AppData, Channel, Message};

use anyhow;
use sled;
use unicode_width::UnicodeWidthStr;

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
        unimplemented!()
    }

    fn load_messages(db: &sled::Db) -> anyhow::Result<Vec<Message>> {
        unimplemented!()
    }

    fn join(channels: Vec<Channel>, messages: Vec<Message>) -> anyhow::Result<AppData> {
        unimplemented!()
    }
}

impl Storage for Db {

    fn save(data: &AppData, path: impl AsRef<Path>) -> anyhow::Result<()> {
        unimplemented!()
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

