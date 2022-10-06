//! Part of the app which is serialized

use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use presage::prelude::proto::data_message::Quote;
use presage::prelude::{GroupMasterKey, GroupSecretParams};
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::receipt::Receipt;
use crate::signal::{Attachment, GroupIdentifierBytes, GroupMasterKeyBytes};
use crate::util::{FilteredStatefulList, StatefulList};

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppData {
    pub channels: FilteredStatefulList<Channel>,
    /// Names retrieved from:
    /// - profiles, when registered as main device)
    /// - contacts, when linked as secondary device
    /// - UUID when both have failed
    ///
    /// Do not use directly, use [`App::name_by_id`] instead.
    pub names: HashMap<Uuid, String>,
    #[serde(default)]
    pub contacts_sync_request_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonChannel")]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub group_data: Option<GroupData>,
    #[serde(serialize_with = "Channel::serialize_msgs")]
    pub messages: StatefulList<Message>,
    pub unread_messages: usize,
    pub typing: TypingSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypingSet {
    SingleTyping(bool),
    GroupTyping(HashSet<Uuid>),
}

/// Proxy type which allows us to apply post-deserialization conversion.
///
/// Used to migrate the schema. Change this type only in backwards-compatible way.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonChannel {
    pub id: ChannelId,
    pub name: String,
    #[serde(default)]
    pub group_data: Option<GroupData>,
    #[serde(
        serialize_with = "Channel::serialize_msgs",
        deserialize_with = "Channel::deserialize_msgs"
    )]
    pub messages: StatefulList<Message>,
    #[serde(default)]
    pub unread_messages: usize,
}

impl TryFrom<JsonChannel> for Channel {
    type Error = anyhow::Error;
    fn try_from(channel: JsonChannel) -> anyhow::Result<Self> {
        let is_group = channel.group_data.is_some();
        let mut channel = Channel {
            id: channel.id,
            name: channel.name,
            group_data: channel.group_data,
            messages: channel.messages,
            unread_messages: channel.unread_messages,
            typing: {
                if is_group {
                    TypingSet::GroupTyping(HashSet::new())
                } else {
                    TypingSet::SingleTyping(false)
                }
            },
        };

        // 1. The master key in ChannelId::Group was replaced by group identifier,
        // the former was stored in group_data.
        match (channel.id, channel.group_data.as_mut()) {
            (ChannelId::Group(id), Some(group_data)) if group_data.master_key_bytes == [0; 32] => {
                group_data.master_key_bytes = id;
                channel.id = ChannelId::from_master_key_bytes(id)?;
            }
            _ => (),
        }
        Ok(channel)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupData {
    #[serde(default)]
    pub master_key_bytes: GroupMasterKeyBytes,
    pub members: Vec<Uuid>,
    pub revision: u32,
}

impl Channel {
    pub fn reset_writing(&mut self, user: Uuid) -> bool {
        match &mut self.typing {
            TypingSet::GroupTyping(ref mut hash_set) => hash_set.remove(&user),
            TypingSet::SingleTyping(true) => {
                self.typing = TypingSet::SingleTyping(false);
                true
            }
            TypingSet::SingleTyping(false) => false,
        }
    }

    pub fn is_writing(&self) -> bool {
        match &self.typing {
            TypingSet::GroupTyping(a) => !a.is_empty(),
            TypingSet::SingleTyping(a) => *a,
        }
    }

    pub fn user_id(&self) -> Option<Uuid> {
        match self.id {
            ChannelId::User(id) => Some(id),
            ChannelId::Group(_) => None,
        }
    }

    pub fn selected_message(&self) -> Option<&Message> {
        // Messages are shown in reversed order => selected is reversed
        self.messages
            .state
            .selected()
            .and_then(|idx| self.messages.items.len().checked_sub(idx + 1))
            .and_then(|idx| self.messages.items.get(idx))
    }

    fn serialize_msgs<S>(messages: &StatefulList<Message>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        // the messages StatefulList becomes the vec that was messages.items
        messages.items.serialize(ser)
    }

    fn deserialize_msgs<'de, D>(deserializer: D) -> Result<StatefulList<Message>, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let tmp: Vec<Message> = serde::de::Deserialize::deserialize(deserializer)?;
        Ok(StatefulList::with_items(tmp))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChannelId {
    User(Uuid),
    Group(GroupIdentifierBytes),
}

impl From<Uuid> for ChannelId {
    fn from(id: Uuid) -> Self {
        ChannelId::User(id)
    }
}

impl PartialEq<Uuid> for ChannelId {
    fn eq(&self, other: &Uuid) -> bool {
        match self {
            ChannelId::User(id) => id == other,
            ChannelId::Group(_) => false,
        }
    }
}

impl ChannelId {
    pub fn from_master_key_bytes(bytes: impl AsRef<[u8]>) -> anyhow::Result<Self> {
        let master_key_ar = bytes
            .as_ref()
            .try_into()
            .map_err(|_| anyhow!("invalid group master key"))?;
        let master_key = GroupMasterKey::new(master_key_ar);
        let secret_params = GroupSecretParams::derive_from_master_key(master_key);
        let group_id = secret_params.get_group_identifier();
        Ok(Self::Group(group_id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TypingAction {
    Started,
    Stopped,
}

impl TypingAction {
    pub fn from_i32(i: i32) -> Self {
        match i {
            0 => Self::Started,
            1 => Self::Stopped,
            _ => {
                error!("Got incorrect TypingAction : {}", i);
                Self::Stopped
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub from_id: Uuid,
    pub message: Option<String>,
    pub arrived_at: u64,
    #[serde(default)]
    pub quote: Option<Box<Message>>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub reactions: Vec<(Uuid, String)>,
    #[serde(default)]
    pub receipt: Receipt,
}

impl Message {
    pub fn new(
        from_id: Uuid,
        message: Option<String>,
        arrived_at: u64,
        attachments: Vec<Attachment>,
    ) -> Self {
        Self {
            from_id,
            message,
            arrived_at,
            quote: None,
            attachments,
            reactions: Default::default(),
            receipt: Receipt::Sent,
        }
    }

    pub fn from_quote(quote: Quote) -> Option<Message> {
        Some(Message {
            from_id: quote.author_uuid?.parse().ok()?,
            message: quote.text,
            arrived_at: quote.id?,
            quote: None,
            attachments: Default::default(),
            reactions: Default::default(),
            receipt: Receipt::Sent,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.message.is_none() && self.attachments.is_empty() && self.reactions.is_empty()
    }
}
