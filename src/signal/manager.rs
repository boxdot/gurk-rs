//! Abstraction of a Signal client

use std::path::PathBuf;
use std::pin::Pin;

use async_trait::async_trait;
use get_size2::GetSize;
use presage::libsignal_service::content::Content;
use presage::libsignal_service::sender::AttachmentSpec;
use presage::model::contacts::Contact;
use presage::model::groups::Group;
use presage::proto::AttachmentPointer;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio_stream::Stream;
use uuid::Uuid;

use crate::data::{Channel, GroupData, Message};
use crate::receipt::Receipt;

use super::{GroupMasterKeyBytes, ProfileKeyBytes};

/// Abstract functionalities of Signal required by the app, that is, dependency inversion
#[async_trait(?Send)]
pub trait SignalManager {
    fn clone_boxed(&self) -> Box<dyn SignalManager>;

    fn user_id(&self) -> Uuid;

    async fn resolve_group(
        &mut self,
        master_key_bytes: GroupMasterKeyBytes,
    ) -> anyhow::Result<ResolvedGroup>;

    async fn save_attachment(
        &mut self,
        attachment_pointer: AttachmentPointer,
    ) -> anyhow::Result<Attachment>;

    fn send_receipt(&self, sender_uuid: Uuid, timestamps: Vec<u64>, receipt: Receipt);

    fn send_text(
        &self,
        channel: &Channel,
        text: String,
        quote_message: Option<&Message>,
        edit_message_timestamp: Option<u64>,
        attachments: Vec<(AttachmentSpec, Vec<u8>)>,
    ) -> (Message, oneshot::Receiver<anyhow::Result<()>>);

    fn send_reaction(&self, channel: &Channel, message: &Message, emoji: String, remove: bool);

    async fn profile_name(&self, id: Uuid) -> Option<String>;

    /// Resolves contact name from user's profile via Signal server
    async fn resolve_profile_name(
        &mut self,
        id: Uuid,
        profile_key: ProfileKeyBytes,
    ) -> Option<String>;

    async fn contact(&self, id: Uuid) -> Option<Contact>;

    async fn receive_messages(
        &mut self,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = Box<Content>>>>>;

    async fn contacts(&self) -> Box<dyn Iterator<Item = Contact>>;
    async fn groups(&self) -> Box<dyn Iterator<Item = (GroupMasterKeyBytes, Group)>>;
}

pub struct ResolvedGroup {
    pub name: String,
    pub group_data: GroupData,
    pub profile_keys: Vec<ProfileKeyBytes>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, GetSize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub content_type: String,
    pub filename: PathBuf,
    pub size: u32,
}
