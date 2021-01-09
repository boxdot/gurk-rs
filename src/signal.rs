use crate::config::Config;

use anyhow::{anyhow, Context as _};
use futures_util::{
    stream::{self, Stream},
    FutureExt, StreamExt,
};
use libsignal_protocol::{crypto::DefaultCrypto, Context};
pub use libsignal_service::content::{AttachmentPointer, ContentBody, DataMessage, Metadata};
use serde::{Deserialize, Serialize};
use signal_bot::{config::SledConfigStore, Manager};
use thiserror::Error;

use std::io::BufRead;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;

#[derive(Clone)]
pub struct SignalClient {
    config: Config,
    manager: Manager<SledConfigStore>,
}

#[derive(Debug)]
pub struct Message {
    pub metadata: Metadata,
    pub body: ContentBody,
}

impl SignalClient {
    pub fn with_config(config: Config) -> Self {
        let config_store = SledConfigStore::new(config.db_path.clone()).unwrap();
        let signal_context = Context::new(DefaultCrypto::default()).unwrap();

        let manager = Manager::with_config_store(config_store, signal_context).unwrap();

        Self { config, manager }
    }

    pub fn get_groups(&self) -> anyhow::Result<Vec<GroupInfo>> {
        // let output = Command::new(self.config.signal_cli.path.as_os_str())
        //     .arg("--username")
        //     .arg(&self.config.user.phone_number)
        //     .arg("listGroups")
        //     .output()?;

        // let res: Result<Vec<_>, anyhow::Error> = output
        //     .stdout
        //     .lines()
        //     .map(|s| {
        //         let s = s?;
        //         let info = GroupInfo::from_str(&s)?;
        //         Ok(info)
        //     })
        //     .collect();
        // res
        Ok(Vec::new())
    }

    pub fn get_contacts(&self) -> anyhow::Result<Vec<ContactInfo>> {
        // let output = Command::new(self.config.signal_cli.path.as_os_str())
        //     .arg("--username")
        //     .arg(&self.config.user.phone_number)
        //     .arg("listContacts")
        //     .output()?;

        // let res: Result<Vec<_>, anyhow::Error> = output
        //     .stdout
        //     .lines()
        //     .map(|s| {
        //         let s = s?;
        //         let info = ContactInfo::from_str(&s)?;
        //         Ok(info)
        //     })
        //     .collect();
        // res
        Ok(Vec::new())
    }

    pub fn stream_messages(self) -> impl Stream<Item = anyhow::Result<Message>> {
        let (tx, rx) = futures::channel::mpsc::channel(32);
        let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        tokio::task::spawn_local(async move {
            let res = match self.manager.receive_messages(tx).await {
                Ok(()) => Err(anyhow!("stopped receiving messages from Signal")),
                Err(e) => Err(e.into()),
            }
            .context("failed to receive a message from Signal");
            stopped_tx.send(res).expect("logic: stopped channel closed");
        });
        stream::select(
            rx.map(|(metadata, body)| Ok(Message { metadata, body })),
            stopped_rx.into_stream().map(|rx_res| rx_res?),
        )
    }

    pub fn send_message(message: &str, phone_number: &str) {
        let child = tokio::process::Command::new("dbus-send")
            .args(&[
                "--session",
                "--type=method_call",
                "--dest=org.asamk.Signal",
                "/org/asamk/Signal",
                "org.asamk.Signal.sendMessage",
            ])
            .arg(format!("string:{}", message))
            .arg("array:string:")
            .arg(format!("string:{}", phone_number))
            .spawn()
            .unwrap();

        tokio::spawn(child);
    }

    pub fn send_group_message(message: &str, group_id: &str) {
        let child = tokio::process::Command::new("dbus-send")
            .args(&[
                "--session",
                "--type=method_call",
                "--dest=org.asamk.Signal",
                "/org/asamk/Signal",
                "org.asamk.Signal.sendGroupMessage",
            ])
            .arg(format!("string:{}", message))
            .arg("array:string:")
            .arg(format!(
                "array:byte:{}",
                bytes_to_decimal_list(&base64::decode(&group_id).unwrap())
            ))
            .spawn()
            .unwrap();

        tokio::spawn(child);
    }

    pub async fn get_contact_name(phone_number: &str) -> Option<String> {
        let output = tokio::process::Command::new("dbus-send")
            .args(&[
                "--session",
                "--print-reply",
                "--type=method_call",
                "--dest=org.asamk.Signal",
                "/org/asamk/Signal",
                "org.asamk.Signal.getContactName",
            ])
            .arg(format!("string:{}", phone_number))
            .output();

        let output = output.await.ok()?;
        extract_dbus_string_response(&output.stdout).filter(|s| !String::is_empty(s))
    }

    pub async fn get_group_name(group_id: &str) -> Option<String> {
        let output = tokio::process::Command::new("dbus-send")
            .args(&[
                "--session",
                "--print-reply",
                "--type=method_call",
                "--dest=org.asamk.Signal",
                "/org/asamk/Signal",
                "org.asamk.Signal.getGroupName",
            ])
            .arg(format!(
                "array:byte:{}",
                bytes_to_decimal_list(&base64::decode(group_id.as_bytes()).ok()?)
            ))
            .output();

        let output = output.await.ok()?;
        extract_dbus_string_response(&output.stdout).filter(|s| !String::is_empty(s))
    }
}

fn bytes_to_decimal_list(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut bytes_list = String::new();
    write!(&mut bytes_list, "{}", bytes[0]).unwrap();
    for byte in &bytes[1..] {
        write!(&mut bytes_list, ",{}", byte).unwrap();
    }
    bytes_list
}

fn extract_dbus_string_response(s: &[u8]) -> Option<String> {
    // TODO: super ugly code to get a value from second line between quotes!
    let line = s.lines().nth(1)?.ok()?;
    let start = line.find('"')?;
    let end = start + 1 + line[start + 1..].find('"')?;
    let response = line[start + 1..end].trim();
    Some(response.to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub source: String,
    pub source_device: u8,
    // pub relay: Option<?>,
    pub timestamp: u64,
    pub message: Option<String>,
    pub is_receipt: bool,
    pub is_read: Option<bool>,
    pub is_delivery: Option<bool>,
    pub data_message: Option<InnerMessage>,
    pub sync_message: Option<SyncMessage>,
    // call_message
    // receipt_message
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InnerMessage {
    pub timestamp: u64,
    pub message: Option<String>,
    pub expires_in_seconds: u64,
    pub attachments: Option<Vec<Attachment>>,
    pub group_info: Option<GroupInfo>,
    pub destination: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncMessage {
    pub sent_message: InnerMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    pub group_id: String,
    // members
    pub name: Option<String>,
    pub members: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub content_type: String,
    pub filename: PathBuf,
    pub size: u64,
}

#[derive(Debug)]
pub struct ContactInfo {
    pub name: String,
    pub phone_number: String,
}

// TODO: robust parsing
impl FromStr for GroupInfo {
    type Err = ParseInfoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use ParseInfoError::*;

        // group id
        if !s.starts_with("Id: ") {
            return Err(UnexpectedCharAt(0));
        }
        let s = &s[4..];
        let pos = s.find("Name: ").ok_or(UnexpectedCharAt(4))?;
        let group_id = s[..pos].trim();
        let s = &s[pos + 6..];

        // name
        let pos = s.find("Active: ").ok_or(UnexpectedCharAt(pos))?;
        let name = s[..pos].trim();

        // TODO: parse rest

        Ok(Self {
            group_id: group_id.to_string(),
            name: Some(name.to_string()),
            members: None,
        })
    }
}

// TODO: robust parsing
impl FromStr for ContactInfo {
    type Err = ParseInfoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use ParseInfoError::*;

        // phone number
        if !s.starts_with("Number: ") {
            return Err(UnexpectedCharAt(0));
        }
        let s = &s[8..];
        let pos = s.find("Name: ").ok_or(UnexpectedCharAt(4))?;
        let phone_number = s[..pos].trim();
        let s = &s[pos + 6..];

        // name
        let pos = s.find("Blocked: ").ok_or(UnexpectedCharAt(pos))?;
        let mut name = s[..pos].trim();
        if name.is_empty() {
            name = phone_number
        }

        // TODO: parse rest

        Ok(Self {
            name: name.to_string(),
            phone_number: phone_number.to_string(),
        })
    }
}

#[derive(Debug, Error)]
pub enum ParseInfoError {
    #[error("unexpected char at: {0}")]
    UnexpectedCharAt(usize),
}
