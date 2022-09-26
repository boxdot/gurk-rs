//! Part of the app which is serialized

use std::collections::{HashMap, HashSet};

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use presage::prelude::proto::data_message::Quote;
use presage::prelude::{GroupMasterKey, GroupSecretParams};
use serde::{ser::SerializeSeq, Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::receipt::Receipt;
use crate::signal::{Attachment, GroupIdentifierBytes, GroupMasterKeyBytes};
use crate::util::{utc_now_timestamp_msec, FilteredStatefulList, SerSkip, StatefulList};

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

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "JsonChannel")]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub group_data: Option<GroupData>,
    #[serde(serialize_with = "Channel::serialize_msgs")]
    pub messages: StatefulList<Message>,
    pub unread_messages: usize,
    pub typing: TypingSet,
    pub expire_timer: Option<u32>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypingSet {
    SingleTyping(bool),
    GroupTyping(HashSet<Uuid>),
}

/// Proxy type which allows us to apply post-deserialization conversion.
///
/// Used to migrate the schema. Change this type only in backwards-compatible way.
#[derive(Deserialize)]
pub struct JsonChannel {
    pub id: ChannelId,
    pub name: String,
    #[serde(default)]
    pub group_data: Option<GroupData>,
    #[serde(deserialize_with = "Channel::deserialize_msgs")]
    pub messages: StatefulList<Message>,
    #[serde(default)]
    pub unread_messages: usize,
    // Default to `None`
    #[serde(default)]
    pub expire_timestamp: Option<u32>,
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
            expire_timer: channel.expire_timestamp,
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

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupData {
    #[serde(default)]
    pub master_key_bytes: GroupMasterKeyBytes,
    pub members: Vec<Uuid>,
    pub revision: u32,
}

impl Channel {
    pub fn reset_writing(&mut self, user: Uuid) {
        match &mut self.typing {
            TypingSet::GroupTyping(ref mut hash_set) => {
                hash_set.remove(&user);
            }
            TypingSet::SingleTyping(_) => {
                self.typing = TypingSet::SingleTyping(false);
            }
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

    fn serialize_msgs<S>(messages: &StatefulList<Message>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        // the messages StatefulList becomes the vec that was messages.items
        let to_write_amount = messages
            .items
            .iter()
            .fold(0, |acc, m| acc + if m.to_skip { 0 } else { 1 });
        let mut seq = serializer.serialize_seq(Some(to_write_amount))?;
        for e in messages.items {
            seq.serialize_element(&e)?;
        }
        seq.end()
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
    /// Whether the message will be skipped when writing the database
    /// and rdrawing the UI
    /// This makes it possible to not remove messages from memory
    /// when they get deleted (e.g. time expiration) but skip them
    /// upon saving the database. This alleviated the need for
    /// numerous useless copy of [`Vec`] because of in-the-middle
    /// deletions.
    #[serde(default)]
    pub to_skip: bool,
    /// The timestamp at which the message should get deleted
    #[serde(default)]
    pub expire_timestamp: ExpireTimer,
}

impl Message {
    pub fn new(
        from_id: Uuid,
        message: Option<String>,
        arrived_at: u64,
        attachments: Vec<Attachment>,
        expire_duration: Option<u32>,
    ) -> Self {
        Self {
            from_id,
            message,
            arrived_at,
            quote: None,
            attachments,
            reactions: Default::default(),
            receipt: Receipt::Sent,
            to_skip: false,
            expire_timestamp: ExpireTimer::from_delay_s_opt(expire_duration),
        }
    }

    pub fn from_quote(quote: Quote, expire_duration: Option<u32>) -> Option<Message> {
        Some(Message {
            from_id: quote.author_uuid?.parse().ok()?,
            message: quote.text,
            arrived_at: quote.id?,
            quote: None,
            attachments: Default::default(),
            reactions: Default::default(),
            receipt: Receipt::Sent,
            to_skip: false,
            expire_timestamp: ExpireTimer::from_delay_s_opt(expire_duration),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.message.is_none() && self.attachments.is_empty() && self.reactions.is_empty()
    }
}

impl SerSkip for Message {
    fn skip(&self) -> bool {
        self.to_skip
    }
}

/// A timestamp representing a message expiration
#[derive(Default, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ExpireTimer(Option<u64>);

impl ExpireTimer {
    pub fn from_delay_s(delay_s: u32) -> Self {
        ExpireTimer(Some(delay_s as u64 * 1_000_000 + utc_now_timestamp_msec()))
    }

    pub fn from_delay_s_opt(delay_s: Option<u32>) -> Self {
        ExpireTimer(delay_s.map(|d| d as u64 * 1_000_000 + utc_now_timestamp_msec()))
    }
}
