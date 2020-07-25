use crate::app::Event;
use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::io::BufRead;
use std::process::Command;
use std::str::FromStr;
use thiserror::Error;

pub struct SignalClient {
    config: Config,
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

        let res: Result<Vec<GroupInfo>, anyhow::Error> = output
            .stdout
            .lines()
            .map(|s| {
                let s = s?;
                let info = GroupInfo::from_str(&s)?;
                Ok(info)
            })
            .collect();
        res
    }

    pub async fn stream_messages<T: std::fmt::Debug>(
        self,
        mut tx: tokio::sync::mpsc::Sender<crate::app::Event<T>>,
    ) -> Result<(), std::io::Error> {
        use std::process::Stdio;
        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut cmd = tokio::process::Command::new(self.config.signal_cli.path);
        cmd.arg("-u")
            .arg(self.config.user.phone_number)
            .arg("daemon")
            .arg("--json")
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn()?;
        let stdout = child
            .stdout
            .take()
            .expect("child did not have a handle to stdout");

        let mut reader = BufReader::new(stdout).lines();
        let cmd_handle = tokio::spawn(async { child.await });

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

    pub fn send_message(message: impl Display, receipient: impl Display) {
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
            .arg(format!("string:{}", receipient))
            .spawn()
            .unwrap();

        tokio::spawn(child);
    }

    pub fn send_group_message(message: impl Display, group_id: &[u8]) {
        use std::fmt::Write;
        let mut bytes_list = String::new();
        write!(&mut bytes_list, "{}", group_id[0]).unwrap();
        for byte in &group_id[1..] {
            write!(&mut bytes_list, ",{}", byte).unwrap();
        }

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
            .arg(format!("array:byte:{}", bytes_list))
            .spawn()
            .unwrap();

        tokio::spawn(child);
    }
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
    pub message: String,
    pub expires_in_seconds: u64,
    // attachments,
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

impl FromStr for GroupInfo {
    type Err = ParseGroupInfoError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use ParseGroupInfoError::*;

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

        Ok(GroupInfo {
            group_id: group_id.to_string(),
            name: Some(name.to_string()),
            members: None,
        })
    }
}

#[derive(Debug, Error)]
pub enum ParseGroupInfoError {
    #[error("unexpected char at: {0}")]
    UnexpectedCharAt(usize),
}
