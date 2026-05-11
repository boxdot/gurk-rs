//! Part of the app which is serialized

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use presage::libsignal_service::zkgroup::groups::{GroupMasterKey, GroupSecretParams};
use presage::proto;
use presage::proto::data_message::Quote;
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
    pub unread_messages: u32,
    pub muted: bool,
    pub typing: TypingSet,
    /// Disappearing messages timer in seconds (None = disabled)
    pub expire_timer: Option<u32>,
}

/// Timeout for typing indicators (10 seconds per Signal protocol)
pub const TYPING_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone)]
pub enum TypingSet {
    /// For direct messages: None = not typing, Some(when) = typing since
    SingleTyping(Option<Instant>),
    /// For groups: maps user UUID to when they started typing
    GroupTyping(HashMap<Uuid, Instant>),
}

impl PartialEq for TypingSet {
    fn eq(&self, other: &Self) -> bool {
        // Compare presence only, not timestamps
        match (self, other) {
            (TypingSet::SingleTyping(a), TypingSet::SingleTyping(b)) => {
                a.is_some() == b.is_some()
            }
            (TypingSet::GroupTyping(a), TypingSet::GroupTyping(b)) => {
                a.len() == b.len() && a.keys().all(|k| b.contains_key(k))
            }
            _ => false,
        }
    }
}

impl Eq for TypingSet {}

impl TypingSet {
    pub fn new(is_group: bool) -> Self {
        if is_group {
            Self::GroupTyping(HashMap::new())
        } else {
            Self::SingleTyping(None)
        }
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
            TypingSet::GroupTyping(map) => map.remove(&user).is_some(),
            TypingSet::SingleTyping(Some(_)) => {
                self.typing = TypingSet::SingleTyping(None);
                true
            }
            TypingSet::SingleTyping(None) => false,
        }
    }

    pub fn is_writing(&self) -> bool {
        match &self.typing {
            TypingSet::GroupTyping(map) => !map.is_empty(),
            TypingSet::SingleTyping(opt) => opt.is_some(),
        }
    }

    /// Remove typing indicators older than the timeout. Returns true if any were removed.
    pub fn expire_typing(&mut self) -> bool {
        let timeout = Duration::from_secs(TYPING_TIMEOUT_SECS);
        let now = Instant::now();

        match &mut self.typing {
            TypingSet::GroupTyping(map) => {
                let before = map.len();
                map.retain(|_, started| now.duration_since(*started) < timeout);
                map.len() != before
            }
            TypingSet::SingleTyping(Some(started)) => {
                if now.duration_since(*started) >= timeout {
                    self.typing = TypingSet::SingleTyping(None);
                    true
                } else {
                    false
                }
            }
            TypingSet::SingleTyping(None) => false,
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

impl TryFrom<&[u8]> for ChannelId {
    type Error = UnexpectedGroupBytesLen;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        bytes
            .try_into()
            .map(ChannelId::Group)
            .map_err(|_| UnexpectedGroupBytesLen(bytes.len()))
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

#[derive(Debug, thiserror::Error)]
#[error("unexpected group bytes length: {0}")]
pub struct UnexpectedGroupBytesLen(usize);

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

    pub(crate) fn user(&self) -> Option<Uuid> {
        match self {
            ChannelId::User(uuid) => Some(*uuid),
            _ => None,
        }
    }

    pub(crate) fn is_user(&self) -> bool {
        matches!(self, ChannelId::User(_))
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
    #[serde(skip)]
    pub(crate) send_failed: Option<String>,
    /// Arrived at of the originally edited message
    ///
    /// When several edits are done, this is the arrived_at of the very first original message.
    #[serde(default)]
    pub(crate) edit: Option<u64>,
    /// Whether the message was edited
    #[serde(default)]
    pub(crate) edited: bool,
    /// Whether the message was deleted (remote delete / delete for everyone)
    #[serde(default)]
    pub(crate) deleted: bool,
    /// Disappearing message timer in seconds (set when message was sent with a timer)
    #[serde(default)]
    pub(crate) expire_timer: Option<u32>,
    /// UTC millisecond timestamp when this message expires (set when first viewed)
    #[serde(default)]
    pub(crate) expires_at: Option<u64>,
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
    Style(Style),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Style {
    #[default]
    None,
    Bold,
    Italic,
    Spoiler,
    Strikethrough,
    Monospace,
}

impl Style {
    fn from_proto(value: proto::body_range::Style) -> Self {
        match value {
            proto::body_range::Style::None => Self::None,
            proto::body_range::Style::Bold => Self::Bold,
            proto::body_range::Style::Italic => Self::Italic,
            proto::body_range::Style::Spoiler => Self::Spoiler,
            proto::body_range::Style::Strikethrough => Self::Strikethrough,
            proto::body_range::Style::Monospace => Self::Monospace,
        }
    }

    fn to_proto(&self) -> proto::body_range::Style {
        match self {
            Style::None => proto::body_range::Style::None,
            Style::Bold => proto::body_range::Style::Bold,
            Style::Italic => proto::body_range::Style::Italic,
            Style::Spoiler => proto::body_range::Style::Spoiler,
            Style::Strikethrough => proto::body_range::Style::Strikethrough,
            Style::Monospace => proto::body_range::Style::Monospace,
        }
    }
}

impl From<&BodyRange> for proto::BodyRange {
    fn from(range: &BodyRange) -> Self {
        let associated_value = match &range.value {
            AssociatedValue::MentionUuid(id) => {
                proto::body_range::AssociatedValue::MentionAci(id.to_string())
            }
            AssociatedValue::Style(style) => {
                proto::body_range::AssociatedValue::Style(style.to_proto().into())
            }
        };
        Self {
            start: Some(range.start.into()),
            length: Some((range.end - range.start).into()),
            associated_value: Some(associated_value),
        }
    }
}

impl BodyRange {
    pub(crate) fn from_proto(proto: proto::BodyRange) -> Option<Self> {
        let value = match proto.associated_value? {
            proto::body_range::AssociatedValue::MentionAci(uuid) => {
                let uuid = uuid.parse().ok()?;
                AssociatedValue::MentionUuid(uuid)
            }
            proto::body_range::AssociatedValue::Style(style) => AssociatedValue::Style(
                proto::body_range::Style::try_from(style)
                    .map(Style::from_proto)
                    .unwrap_or_default(),
            ),
            proto::body_range::AssociatedValue::MentionAciBinary(uuid) => {
                let uuid = Uuid::from_slice(&uuid).ok()?;
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
            send_failed: Default::default(),
            edit: Default::default(),
            edited: Default::default(),
            deleted: Default::default(),
            expire_timer: None,
            expires_at: None,
        }
    }

    pub(crate) fn text(from_id: Uuid, arrived_at: u64, message: String) -> Self {
        Self {
            from_id,
            message: Some(message),
            arrived_at,
            quote: Default::default(),
            attachments: Default::default(),
            reactions: Default::default(),
            receipt: Default::default(),
            body_ranges: Default::default(),
            send_failed: Default::default(),
            edit: Default::default(),
            edited: Default::default(),
            deleted: Default::default(),
            expire_timer: None,
            expires_at: None,
        }
    }

    pub fn from_quote(quote: Quote) -> Option<Message> {
        Some(Message {
            from_id: quote.author_aci?.parse().ok()?,
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
            send_failed: Default::default(),
            edit: Default::default(),
            edited: Default::default(),
            deleted: Default::default(),
            expire_timer: None,
            expires_at: None,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.message.is_none()
            && self.attachments.is_empty()
            && self.reactions.is_empty()
            && self.quote.is_none()
    }
}
