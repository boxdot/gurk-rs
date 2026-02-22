//! Implementation of [`crate::signal::SignalManager`] via `presage`

use std::{path::PathBuf, pin::Pin};

use anyhow::Context;
use async_trait::async_trait;
use presage::libsignal_service::prelude::ProfileKey;
use presage::libsignal_service::protocol::ServiceId;
use presage::libsignal_service::sender::AttachmentSpec;
use presage::manager::Registered;
use presage::model::contacts::Contact;
use presage::model::groups::Group;
use presage::proto::data_message::{Quote, Reaction};
use presage::proto::{AttachmentPointer, DataMessage, EditMessage, GroupContextV2, ReceiptMessage};
use presage::store::ContentsStore;
use presage::{
    libsignal_service::content::{Content, ContentBody},
    model::messages::Received,
};
use presage_store_sqlite::SqliteStore;
use sha2::Digest;
use tokio::sync::oneshot;
use tokio_stream::{Stream, StreamExt};
use tokio_util::task::LocalPoolHandle;
use tracing::{error, warn};
use uuid::Uuid;

use crate::data::{Channel, ChannelId, GroupData, Message};
use crate::receipt::Receipt;
use crate::util::utc_now_timestamp_msec;

use super::{
    Attachment, GroupMasterKeyBytes, ProfileKeyBytes, ResolvedGroup, SignalManager, attachment,
};

pub(super) struct PresageManager {
    manager: presage::Manager<SqliteStore, Registered>,
    data_dir: PathBuf,
    local_pool: LocalPoolHandle,
}

impl PresageManager {
    pub(super) fn new(
        manager: presage::Manager<SqliteStore, Registered>,
        data_dir: PathBuf,
        local_pool: LocalPoolHandle,
    ) -> Self {
        Self {
            manager,
            data_dir,
            local_pool,
        }
    }
}

#[async_trait(?Send)]
impl SignalManager for PresageManager {
    fn clone_boxed(&self) -> Box<dyn SignalManager> {
        Box::new(Self::new(
            self.manager.clone(),
            self.data_dir.clone(),
            self.local_pool.clone(),
        ))
    }

    fn user_id(&self) -> Uuid {
        self.manager.registration_data().service_ids.aci
    }

    async fn resolve_group(
        &mut self,
        master_key_bytes: GroupMasterKeyBytes,
    ) -> anyhow::Result<ResolvedGroup> {
        let decrypted_group = self
            .manager
            .store()
            .group(master_key_bytes)
            .await?
            .context("no group found")?;

        let mut members = Vec::with_capacity(decrypted_group.members.len());
        let mut profile_keys = Vec::with_capacity(decrypted_group.members.len());
        for member in decrypted_group.members {
            members.push(member.aci.into());
            profile_keys.push(member.profile_key.bytes);
        }

        let name = decrypted_group.title;
        let group_data = GroupData {
            master_key_bytes,
            members,
            revision: decrypted_group.revision,
        };

        Ok(ResolvedGroup {
            name,
            group_data,
            profile_keys,
        })
    }

    async fn save_attachment(
        &mut self,
        attachment_pointer: AttachmentPointer,
    ) -> anyhow::Result<Attachment> {
        let attachment_data = self.manager.get_attachment(&attachment_pointer).await?;
        attachment::save(&self.data_dir, attachment_pointer, &attachment_data)
    }

    fn send_receipt(&self, sender_uuid: Uuid, timestamps: Vec<u64>, receipt: Receipt) {
        let now_timestamp = utc_now_timestamp_msec();
        let data_message = ReceiptMessage {
            r#type: Some(receipt.to_i32()),
            timestamp: timestamps,
        };

        let mut manager = self.manager.clone();
        self.local_pool.spawn_pinned(move || async move {
            let body = ContentBody::ReceiptMessage(data_message);
            if let Err(error) = manager
                .send_message(ServiceId::Aci(sender_uuid.into()), body, now_timestamp)
                .await
            {
                error!(%error, %sender_uuid, "failed to send receipt");
            }
        });
    }

    fn send_text(
        &self,
        channel: &Channel,
        text: String,
        quote_message: Option<&Message>,
        edit_message_timestamp: Option<u64>,
        attachments: Vec<(AttachmentSpec, Vec<u8>)>,
    ) -> (Message, oneshot::Receiver<anyhow::Result<()>>) {
        let message: String = crate::emoji::replace_shortcodes(&text).into_owned();
        let has_attachments = !attachments.is_empty();

        let timestamp = utc_now_timestamp_msec();

        let quote = quote_message.map(|message| Quote {
            id: Some(message.arrived_at),
            author_aci: Some(message.from_id.to_string()),
            text: message.message.clone(),
            body_ranges: message.body_ranges.iter().map(From::from).collect(),
            ..Default::default()
        });
        let quote_message = quote.clone().and_then(Message::from_quote).map(Box::new);

        let mut data_message = DataMessage {
            body: Some(message.clone()),
            quote,
            ..Default::default()
        };

        let mut saved_attachments: Vec<Attachment> = Vec::new();
        if has_attachments {
            for (spec, data) in &attachments {
                let attachment_pointer = AttachmentPointer {
                    content_type: Some(spec.content_type.clone()),
                    size: Some(spec.length as u32),
                    digest: Some(sha2::Sha256::digest(data).to_vec()),
                    file_name: spec.file_name.clone(),
                    width: spec.width,
                    height: spec.height,
                    upload_timestamp: Some(utc_now_timestamp_msec()),
                    ..Default::default()
                };
                match attachment::save(&self.data_dir, attachment_pointer, data) {
                    Ok(attachment) => {
                        saved_attachments.push(attachment);
                    }
                    Err(error) => {
                        error!(%error, "failed to save attachment");
                    }
                }
            }
        }

        let (response_tx, response) = oneshot::channel();
        match channel.id {
            ChannelId::User(uuid) => {
                let mut manager = self.manager.clone();
                self.local_pool.spawn_pinned(move || async move {
                    if let Err(error) =
                        upload_attachments(&manager, attachments, &mut data_message).await
                    {
                        error!(%error, "failed to upload attachments");
                        let _ = response_tx.send(Err(error));
                        return;
                    }

                    let body = if let Some(target_sent_timestamp) = edit_message_timestamp {
                        ContentBody::EditMessage(EditMessage {
                            target_sent_timestamp: Some(target_sent_timestamp),
                            data_message: Some(data_message),
                        })
                    } else {
                        ContentBody::DataMessage(data_message)
                    };

                    if let Err(error) = manager
                        .send_message(ServiceId::Aci(uuid.into()), body, timestamp)
                        .await
                    {
                        error!(dest =% uuid, %error, "failed to send message");
                        let _ = response_tx.send(Err(error.into()));
                        return;
                    }
                    let _ = response_tx.send(Ok(()));
                });
            }
            ChannelId::Group(_) => {
                if let Some(group_data) = channel.group_data.as_ref() {
                    let mut manager = self.manager.clone();

                    let master_key_bytes = group_data.master_key_bytes.to_vec();
                    data_message.group_v2 = Some(GroupContextV2 {
                        master_key: Some(master_key_bytes.clone()),
                        revision: Some(group_data.revision),
                        ..Default::default()
                    });

                    self.local_pool.spawn_pinned(move || async move {
                        if let Err(error) =
                            upload_attachments(&manager, attachments, &mut data_message).await
                        {
                            error!(%error, "failed to upload attachments");
                            let _ = response_tx.send(Err(error));
                            return;
                        }

                        let body = if let Some(target_sent_timestamp) = edit_message_timestamp {
                            ContentBody::EditMessage(EditMessage {
                                target_sent_timestamp: Some(target_sent_timestamp),
                                data_message: Some(data_message),
                            })
                        } else {
                            ContentBody::DataMessage(data_message)
                        };

                        if let Err(error) = manager
                            .send_message_to_group(&master_key_bytes, body, timestamp)
                            .await
                        {
                            error!(%error, "failed to send group message");
                            let _ = response_tx.send(Err(error.into()));
                            return;
                        }
                        let _ = response_tx.send(Ok(()));
                    });
                } else {
                    error!("cannot send to broken channel without group data");
                }
            }
        }

        let message = Message {
            from_id: self.user_id(),
            message: Some(message),
            arrived_at: timestamp,
            quote: quote_message,
            attachments: saved_attachments,
            reactions: Default::default(),
            receipt: Receipt::Sent,
            body_ranges: Default::default(),
            send_failed: Default::default(),
            edit: edit_message_timestamp,
            edited: edit_message_timestamp.is_some(),
        };
        (message, response)
    }

    fn send_reaction(&self, channel: &Channel, message: &Message, emoji: String, remove: bool) {
        let timestamp = utc_now_timestamp_msec();
        let target_author_uuid = message.from_id;
        let target_sent_timestamp = message.arrived_at;

        let mut data_message = DataMessage {
            reaction: Some(Reaction {
                emoji: Some(emoji.clone()),
                remove: Some(remove),
                target_author_aci: Some(target_author_uuid.to_string()),
                target_sent_timestamp: Some(target_sent_timestamp),
            }),
            ..Default::default()
        };

        match (channel.id, channel.group_data.as_ref()) {
            (ChannelId::User(uuid), _) => {
                let mut manager = self.manager.clone();
                let body = ContentBody::DataMessage(data_message);
                self.local_pool.spawn_pinned(move || async move {
                    if let Err(e) = manager
                        .send_message(ServiceId::Aci(uuid.into()), body, timestamp)
                        .await
                    {
                        // TODO: Proper error handling
                        error!("failed to send reaction {} to {}: {}", &emoji, uuid, e);
                    }
                });
            }
            (ChannelId::Group(_), Some(group_data)) => {
                let mut manager = self.manager.clone();

                let master_key_bytes = group_data.master_key_bytes.to_vec();
                data_message.group_v2 = Some(GroupContextV2 {
                    master_key: Some(master_key_bytes.clone()),
                    revision: Some(group_data.revision),
                    ..Default::default()
                });

                self.local_pool.spawn_pinned(move || async move {
                    if let Err(e) = manager
                        .send_message_to_group(&master_key_bytes, data_message, timestamp)
                        .await
                    {
                        // TODO: Proper error handling
                        error!("failed to send group reaction {}: {}", &emoji, e);
                    }
                });
            }
            _ => {
                error!("cannot send to broken channel without group data");
            }
        }
    }

    async fn resolve_profile_name(
        &mut self,
        id: Uuid,
        profile_key: ProfileKeyBytes,
    ) -> Option<String> {
        match self
            .manager
            .retrieve_profile_by_uuid(id, ProfileKey::create(profile_key))
            .await
        {
            Ok(profile) => Some(profile.name?.given_name),
            Err(e) => {
                error!("failed to retrieve user profile: {}", e);
                None
            }
        }
    }

    async fn profile_name(&self, id: Uuid) -> Option<String> {
        let service_id = ServiceId::Aci(id.into());
        let profile_key = self.manager.store().profile_key(&service_id).await.ok()??;
        let profile = self.manager.store().profile(id, profile_key).await.ok()??;
        let given_name = profile.name?.given_name;
        if !given_name.is_empty() {
            Some(given_name)
        } else {
            None
        }
    }

    async fn contact(&self, id: Uuid) -> Option<Contact> {
        self.manager.store().contact_by_id(&id).await.ok()?
    }

    async fn receive_messages(
        &mut self,
    ) -> anyhow::Result<Pin<Box<dyn Stream<Item = Box<Content>>>>> {
        Ok(Box::pin(self.manager.receive_messages().await?.filter_map(
            |received| match received {
                Received::Content(content) => Some(content),
                Received::QueueEmpty => None,
                Received::Contacts => {
                    // TODO: <https://github.com/boxdot/gurk-rs/issues/349>
                    warn!("Received contacts, but not implemented yet");
                    None
                }
            },
        )))
    }

    async fn contacts(&self) -> Box<dyn Iterator<Item = Contact>> {
        Box::new(
            self.manager
                .store()
                .contacts()
                .await
                .into_iter()
                .flatten()
                .flatten(),
        )
    }

    async fn groups(&self) -> Box<dyn Iterator<Item = (GroupMasterKeyBytes, Group)>> {
        Box::new(
            self.manager
                .store()
                .groups()
                .await
                .into_iter()
                .flatten()
                .flatten(),
        )
    }
}

async fn upload_attachments(
    manager: &presage::Manager<SqliteStore, Registered>,
    attachments: Vec<(AttachmentSpec, Vec<u8>)>,
    data_message: &mut DataMessage,
) -> anyhow::Result<()> {
    let attachment_pointers = manager.upload_attachments(attachments).await?;
    data_message.attachments = attachment_pointers
        .into_iter()
        .filter_map(|res| {
            if let Err(e) = res.as_ref() {
                error!("failed to upload attachment: {}", e);
            }
            res.ok()
        })
        .collect();
    Ok(())
}
