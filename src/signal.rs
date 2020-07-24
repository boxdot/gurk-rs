use crate::config::Config;
use serde::{Deserialize, Serialize};
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
    pub data_message: Option<DataMessage>,
    // sync_message
    // call_message
    // receipt_message
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMessage {
    pub timestamp: u64,
    pub message: String,
    pub expires_in_seconds: u64,
    // attachments,
    pub group_info: Option<GroupInfo>,
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
