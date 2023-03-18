//! Part of the app which is serialized

use std::collections::HashSet;

use anyhow::anyhow;
use presage::prelude::proto::data_message::{self, Quote};
use presage::prelude::{GroupMasterKey, GroupSecretParams};
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::receipt::Receipt;
use crate::signal::{Attachment, GroupIdentifierBytes, GroupMasterKeyBytes};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub group_data: Option<GroupData>,
    pub unread_messages: usize,
    pub typing: TypingSet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypingSet {
    SingleTyping(bool),
    GroupTyping(HashSet<Uuid>),
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
    #[serde(default)]
    pub(crate) body_ranges: Vec<BodyRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BodyRange {
    pub(crate) start: u16,
    pub(crate) end: u16,
    pub(crate) value: AssociatedValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AssociatedValue {
    MentionUuid(Uuid),
}

impl From<&BodyRange> for data_message::BodyRange {
    fn from(range: &BodyRange) -> Self {
        match range.value {
            AssociatedValue::MentionUuid(id) => Self {
                start: Some(range.start.into()),
                length: Some((range.end - range.start).into()),
                associated_value: Some(data_message::body_range::AssociatedValue::MentionUuid(
                    id.to_string(),
                )),
            },
        }
    }
}

impl BodyRange {
    pub(crate) fn from_proto(proto: data_message::BodyRange) -> Option<Self> {
        let value = match proto.associated_value? {
            data_message::body_range::AssociatedValue::MentionUuid(uuid) => {
                let uuid = uuid.parse().ok()?;
                AssociatedValue::MentionUuid(uuid)
            }
        };
        Some(Self {
            start: proto.start?.try_into().ok()?,
            end: (proto.start? + proto.length?).try_into().ok()?,
            value,
        })
    }
}

impl Message {
    pub(crate) fn new(
        from_id: Uuid,
        message: Option<String>,
        body_ranges: impl IntoIterator<Item = BodyRange>,
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
            body_ranges: body_ranges.into_iter().collect(),
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
            body_ranges: quote
                .body_ranges
                .into_iter()
                .filter_map(BodyRange::from_proto)
                .collect(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.message.is_none() && self.attachments.is_empty() && self.reactions.is_empty()
    }
}
