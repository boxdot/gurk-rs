//! Abstraction of a Signal client

use std::path::PathBuf;
use std::pin::Pin;

use async_trait::async_trait;
use presage::prelude::proto::AttachmentPointer;
use presage::prelude::{AttachmentSpec, Contact, Content};
use serde::{Deserialize, Serialize};
use tokio_stream::Stream;
use uuid::Uuid;

use crate::data::{Channel, GroupData, Message};
use crate::receipt::Receipt;

use super::{GroupMasterKeyBytes, ProfileKey};

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
        attachments: Vec<(AttachmentSpec, Vec<u8>)>,
    ) -> Message;

    fn send_reaction(&self, channel: &Channel, message: &Message, emoji: String, remove: bool);

    /// Resolves contact name from user's profile via Signal server
    async fn resolve_name_from_profile(&self, id: Uuid, profile_key: ProfileKey) -> Option<String>;

    async fn request_contacts_sync(&self) -> anyhow::Result<()>;

    /// Retrieves contact information stored in the manager
    ///
    /// The information is based on the contact book of the client and is only available after
    /// [`Self::request_contacts_sync`] was called **and** contacts where received from Signal server.
    /// This usually happens shortly after the latter method is called.
    fn contact_by_id(&self, id: Uuid) -> anyhow::Result<Option<Contact>>;

    async fn receive_messages(&mut self) -> anyhow::Result<Pin<Box<dyn Stream<Item = Content>>>>;
}

pub struct ResolvedGroup {
    pub name: String,
    pub group_data: GroupData,
    pub profile_keys: Vec<ProfileKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub content_type: String,
    pub filename: PathBuf,
    pub size: u32,
}
