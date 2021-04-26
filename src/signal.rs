use crate::app::{self, Event};
use crate::config::{self, Config};

use anyhow::Context;
use log::error;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

use std::io::BufRead;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct SignalClient {
    config: Config,
}

/// Signal Manager backed by a `sled` store.
pub type Manager = presage::Manager<presage::config::SledConfigStore>;

fn get_signal_manager() -> anyhow::Result<Manager> {
    let data_dir = config::default_data_dir();
    let db_path = data_dir.join("signal-db");
    let config_store = presage::config::SledConfigStore::new(db_path)?;
    let signal_context =
        libsignal_protocol::Context::new(libsignal_protocol::crypto::DefaultCrypto::default())?;
    let manager = presage::Manager::with_config_store(config_store, signal_context)?;
    Ok(manager)
}

pub async fn ensure_linked_device() -> anyhow::Result<(Manager, Config)> {
    let mut manager = get_signal_manager()?;
    let config = if let Some(config_path) = config::installed_config() {
        config::load_from(config_path)?
    } else {
        if manager.phone_number().is_none() {
            // link device
            let at_hostname = hostname::get()
                .ok()
                .and_then(|hostname| {
                    hostname
                        .to_string_lossy()
                        .split('.')
                        .filter(|s| !s.is_empty())
                        .next()
                        .map(|s| format!("@{}", s))
                })
                .unwrap_or_else(String::new);
            let device_name = format!("gurk{}", at_hostname);
            println!("Linking new device with device name: {}", device_name);
            manager
                .link_secondary_device(
                    libsignal_service::configuration::SignalServers::Production,
                    device_name.clone(),
                )
                .await?;
        }

        let phone_number = manager
            .phone_number()
            .expect("no phone number after device was linked");
        let profile = manager.retrieve_profile().await?;
        let name = profile
            .name
            .map(|name| name.given_name)
            .unwrap_or_else(|| whoami::username());

        let user = config::User {
            name,
            phone_number: phone_number.to_string(),
        };
        let config = config::Config::with_user(user);
        config.save_new().context("failed to init config file")?;

        config
    };

    Ok((manager, config))
}

impl SignalClient {
    pub fn from_config(config: Config) -> Self {
        Self { config }
    }

    pub fn get_groups(&self) -> anyhow::Result<Vec<GroupInfo>> {
        let output = Command::new(self.config.signal_cli.path.as_os_str())
            .arg("--username")
            .arg(&self.config.user.phone_number)
            .arg("listGroups")
            .output()?;

        let res: Result<Vec<_>, anyhow::Error> = output
            .stdout
            .lines()
            .map(|s| {
                let s = s?;
                let info = GroupInfo::from_str(&s)?;
                Ok(info)
            })
            .collect();
        log::debug!("got groups: {:?}", res);

        res
    }

    pub fn get_contacts(&self) -> anyhow::Result<Vec<ContactInfo>> {
        let output = Command::new(self.config.signal_cli.path.as_os_str())
            .arg("--username")
            .arg(&self.config.user.phone_number)
            .arg("listContacts")
            .output()?;

        let res: Result<Vec<_>, anyhow::Error> = output
            .stdout
            .lines()
            .map(|s| {
                let s = s?;
                let info = ContactInfo::from_str(&s)?;
                Ok(info)
            })
            .collect();
        log::debug!("got contacts: {:?}", res);

        res
    }

    pub async fn stream_messages<T: std::fmt::Debug, C: std::fmt::Debug>(
        self,
        tx: mpsc::Sender<app::Event>,
    ) -> Result<(), std::io::Error> {
        use std::process::Stdio;
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut cmd = tokio::process::Command::new(self.config.signal_cli.path);
        cmd.arg("-u")
            .arg(self.config.user.phone_number)
            .arg("--output=json")
            .arg("daemon")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .expect("child did not have a handle to stdout");

        let mut reader = BufReader::new(stdout).lines();
        let cmd_handle = tokio::spawn(async move { child.wait().await });

        while let Some(payload) = reader.next_line().await? {
            let message = serde_json::from_str(&payload).ok();
            if tx.send(Event::Message { payload, message }).await.is_err() {
                break; // receiver closed
            }
        }

        // wait until child process stops
        cmd_handle.await??;

        Ok(())
    }

    pub fn send_message(message: &str, phone_number: &str) {
        let mut child = tokio::process::Command::new("dbus-send")
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

        tokio::spawn(async move { child.wait().await });
    }

    pub fn send_group_message(message: &str, group_id: &str) {
        let mut child = tokio::process::Command::new("dbus-send")
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

        tokio::spawn(async move { child.wait().await });
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
pub struct Message {
    pub envelope: Envelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub source: String,
    pub source_device: u8,
    // pub relay: Option<?>,
    pub timestamp: u64,
    pub message: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_deser() {
        let value = serde_json::json!({
            "envelope": {
                "source": "+010000000000",
                "sourceDevice": 1,
                "relay": null,
                "timestamp": 1606502956755_u64,
                "dataMessage": null,
                "syncMessage": {
                    "sentMessage": {
                        "timestamp": 1606502956755_u64,
                        "message": "foobar",
                        "expiresInSeconds": 0,
                        "attachments": [],
                        "groupInfo": null,
                        "destination": "+010000000000"
                    },
                    "blockedNumbers": null,
                    "readMessages": null,
                    "type": null
                },
                "callMessage": null,
                "receiptMessage": null
            }
        });
        let _: Message = serde_json::from_value(value).expect("failed to deserialize message");
    }
}
