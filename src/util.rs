use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tui::widgets::ListState;

/**
 * To store current invites
 */
pub struct OutgoingInvite {
    pub account: String,
    pub channel: Option<String>,
    pub member: String,
}

/**
 * To store pending conversation member removal
 */
pub struct PendingRm {
    pub account: String,
    pub channel: String,
    pub member: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Role {
    Member,
    Admin,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Member {
    pub hash: String,
    pub role: Role,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChannelType {
    Generated,
    Group,
    Invite,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    /// Either phone number or group id
    pub id: String,
    pub name: String,
    pub channel_type: ChannelType,
    pub members: Vec<Member>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub unread_messages: usize,
}

impl Channel {
    pub fn new(id: &String, channel_type: ChannelType) -> Channel {
        let mut name = id.clone();
        if channel_type == ChannelType::Invite {
            name = String::from(format!("🔵 {}", id));
        }
        Channel {
            id: id.clone(),
            name: name,
            members: Vec::new(),
            channel_type,
            messages: Vec::new(),
            unread_messages: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub from: String,
    #[serde(alias = "text")] // remove
    pub message: String,
    pub arrived_at: DateTime<Utc>,
}

impl Message {
    pub fn info(message: String) -> Message {
        Message {
            from: String::new(),
            message,
            arrived_at: Utc::now()
        }
    }

    pub fn new(from: String, message: String, arrived_at: DateTime<Utc>) -> Message {
        Message {
            from,
            message,
            arrived_at
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event<I> {
    Input(I),
    Message {
        account_id: String,
        conversation_id: String,
        payloads: HashMap<String, String>,
    },
    ConversationReady(String, String),
    ConversationRequest(String, String),
    RegistrationStateChanged(String, String),
    ProfileReceived(String, String, String),
    RegisteredNameFound(String, u64, String, String),
    AccountsChanged(),
    ConversationLoaded(u32, String, String, Vec<HashMap<String, String>>),
    Resize,
}

#[derive(Serialize, Deserialize)]
pub struct StatefulList<T> {
    #[serde(skip)]
    pub state: ListState,
    pub items: Vec<T>,
}

impl<T> StatefulList<T> {
    pub fn with_items(items: Vec<T>) -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items,
        }
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i + 1 >= self.items.len() {
                    0
                } else {
                    i + 1
                }
            }
            None => {
                if !self.items.is_empty() {
                    0
                } else {
                    return; // nothing to select
                }
            }
        };
        self.state.select(Some(i));
    }

    pub fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => {
                if !self.items.is_empty() {
                    0
                } else {
                    return; // nothing to select
                }
            }
        };
        self.state.select(Some(i));
    }
}
