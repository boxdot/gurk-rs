use crate::app::{Channel, ChannelId, GroupData, Message, Receipt};
use crate::config::{self, Config};
use crate::util::utc_now_timestamp_msec;

use anyhow::anyhow;
use anyhow::{bail, Context as _};
use async_trait::async_trait;
use chrono::Utc;
use gh_emoji::Replacer;
use presage::prelude::content::Reaction;
use presage::prelude::proto::data_message::Quote;
use presage::prelude::proto::{AttachmentPointer, ReceiptMessage};
use presage::prelude::{
    AttachmentSpec, Contact, ContentBody, DataMessage, GroupContextV2, GroupMasterKey,
    SignalServers,
};
use serde::{Deserialize, Serialize};
use tracing::{error, warn};
use uuid::Uuid;

use std::path::PathBuf;

// TODO: these should be either re-exported from presage/libsignal-service
pub const PROFILE_KEY_LEN: usize = 32;
pub const GROUP_MASTER_KEY_LEN: usize = 32;
pub const GROUP_IDENTIFIER_LEN: usize = 32;

pub type ProfileKey = [u8; PROFILE_KEY_LEN];
pub type GroupMasterKeyBytes = [u8; GROUP_MASTER_KEY_LEN];
pub type GroupIdentifierBytes = [u8; GROUP_IDENTIFIER_LEN];

/// Signal Manager backed by a `sled` store.
pub type Manager = presage::Manager<presage::SledConfigStore>;

#[async_trait(?Send)]
pub trait SignalManager {
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

    /// Resolves contact name from its profile
    async fn resolve_name_from_profile(&self, id: Uuid, profile_key: ProfileKey) -> Option<String>;

    async fn request_contacts_sync(&self) -> anyhow::Result<()>;

    /// Retrieves contact information store in the manager
    ///
    /// The information is based on the contact book of the client and is only available after
    /// [`request_contacts_sync`] was called **and** contacts where received from Signal server.
    /// This usually happens shortly after the latter method is called.
    fn contact_by_id(&self, id: Uuid) -> anyhow::Result<Option<Contact>>;
}

pub struct ResolvedGroup {
    pub name: String,
    pub group_data: GroupData,
    pub profile_keys: Vec<ProfileKey>,
}

pub struct PresageManager {
    manager: Manager,
    emoji_replacer: Replacer,
}

impl PresageManager {
    pub fn new(manager: Manager) -> Self {
        Self {
            manager,
            emoji_replacer: Replacer::new(),
        }
    }
}

#[async_trait(?Send)]
impl SignalManager for PresageManager {
    fn user_id(&self) -> Uuid {
        self.manager.uuid()
    }

    fn send_receipt(&self, _sender_uuid: Uuid, timestamps: Vec<u64>, receipt: Receipt) {
        let now_timestamp = utc_now_timestamp_msec();
        let data_message = ReceiptMessage {
            r#type: Some(receipt.to_i32()),
            timestamp: timestamps,
        };

        let manager = self.manager.clone();
        tokio::task::spawn_local(async move {
            let body = ContentBody::ReceiptMessage(data_message);
            if let Err(e) = manager
                .send_message(_sender_uuid, body, now_timestamp)
                .await
            {
                error!("Failed to send message to {}: {}", _sender_uuid, e);
            }
        });
    }

    fn send_text(
        &self,
        channel: &Channel,
        text: String,
        quote_message: Option<&Message>,
        attachments: Vec<(AttachmentSpec, Vec<u8>)>,
    ) -> Message {
        let mut message: String = self.emoji_replacer.replace_all(&text).into_owned();
        let has_attachments = !attachments.is_empty();

        let timestamp = utc_now_timestamp_msec();

        let quote = quote_message.map(|message| Quote {
            id: Some(message.arrived_at),
            author_uuid: Some(message.from_id.to_string()),
            text: message.message.clone(),
            ..Default::default()
        });
        let quote_message = quote.clone().and_then(Message::from_quote).map(Box::new);

        let mut data_message = DataMessage {
            body: Some(message.clone()),
            timestamp: Some(timestamp),
            quote,
            ..Default::default()
        };

        match channel.id {
            ChannelId::User(uuid) => {
                let manager = self.manager.clone();
                tokio::task::spawn_local(async move {
                    upload_attachments(&manager, attachments, &mut data_message).await;

                    let body = ContentBody::DataMessage(data_message);
                    if let Err(e) = manager.send_message(uuid, body, timestamp).await {
                        // TODO: Proper error handling
                        error!("Failed to send message to {}: {}", uuid, e);
                    }
                });
            }
            ChannelId::Group(_) => {
                if let Some(group_data) = channel.group_data.as_ref() {
                    let manager = self.manager.clone();
                    let self_uuid = self.user_id();

                    data_message.group_v2 = Some(GroupContextV2 {
                        master_key: Some(group_data.master_key_bytes.to_vec()),
                        revision: Some(group_data.revision),
                        ..Default::default()
                    });

                    let recipients = group_data.members.clone().into_iter();

                    tokio::task::spawn_local(async move {
                        upload_attachments(&manager, attachments, &mut data_message).await;

                        let recipients =
                            recipients.filter(|uuid| *uuid != self_uuid).map(Into::into);
                        if let Err(e) = manager
                            .send_message_to_group(recipients, data_message, timestamp)
                            .await
                        {
                            // TODO: Proper error handling
                            error!("Failed to send group message: {}", e);
                        }
                    });
                } else {
                    error!("cannot send to broken channel without group data");
                }
            }
        }

        if has_attachments && message.is_empty() {
            // TODO: Temporary solution until we start rendering attachments
            message = "<attachment>".to_string();
        }

        Message {
            from_id: self.user_id(),
            message: Some(message),
            arrived_at: timestamp,
            quote: quote_message,
            attachments: Default::default(),
            reactions: Default::default(),
            receipt: Receipt::Sent,
        }
    }

    fn send_reaction(&self, channel: &Channel, message: &Message, emoji: String, remove: bool) {
        let timestamp = utc_now_timestamp_msec();
        let target_author_uuid = message.from_id;
        let target_sent_timestamp = message.arrived_at;

        let mut data_message = DataMessage {
            reaction: Some(Reaction {
                emoji: Some(emoji.clone()),
                remove: Some(remove),
                target_author_uuid: Some(target_author_uuid.to_string()),
                target_sent_timestamp: Some(target_sent_timestamp),
            }),
            ..Default::default()
        };

        match (channel.id, channel.group_data.as_ref()) {
            (ChannelId::User(uuid), _) => {
                let manager = self.manager.clone();
                let body = ContentBody::DataMessage(data_message);
                tokio::task::spawn_local(async move {
                    if let Err(e) = manager.send_message(uuid, body, timestamp).await {
                        // TODO: Proper error handling
                        error!("failed to send reaction {} to {}: {}", &emoji, uuid, e);
                    }
                });
            }
            (ChannelId::Group(_), Some(group_data)) => {
                let manager = self.manager.clone();
                let self_uuid = self.user_id();

                data_message.group_v2 = Some(GroupContextV2 {
                    master_key: Some(group_data.master_key_bytes.to_vec()),
                    revision: Some(group_data.revision),
                    ..Default::default()
                });

                let recipients = group_data.members.clone().into_iter();

                tokio::task::spawn_local(async move {
                    let recipients = recipients.filter(|uuid| *uuid != self_uuid).map(Into::into);
                    if let Err(e) = manager
                        .send_message_to_group(recipients, data_message, timestamp)
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

    async fn resolve_name_from_profile(&self, id: Uuid, profile_key: ProfileKey) -> Option<String> {
        match self.manager.retrieve_profile_by_uuid(id, profile_key).await {
            Ok(profile) => Some(profile.name?.given_name),
            Err(e) => {
                error!("failed to retrieve user profile: {}", e);
                None
            }
        }
    }

    async fn resolve_group(
        &mut self,
        master_key_bytes: GroupMasterKeyBytes,
    ) -> anyhow::Result<ResolvedGroup> {
        let master_key = GroupMasterKey::new(master_key_bytes);
        let decrypted_group = self.manager.get_group_v2(master_key).await?;

        let mut members = Vec::with_capacity(decrypted_group.members.len());
        let mut profile_keys = Vec::with_capacity(decrypted_group.members.len());
        for member in decrypted_group.members {
            let uuid = match Uuid::from_slice(&member.uuid) {
                Ok(id) => id,
                Err(_) => continue,
            };
            members.push(uuid);
            profile_keys.push(
                member
                    .profile_key
                    .try_into()
                    .map_err(|_| anyhow!("malformed profile key"))?,
            );
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
        let data_dir = dirs::data_dir()
            .ok_or_else(|| anyhow!("could not find data directory"))?
            .join("gurk");
        let attachment_data = self.manager.get_attachment(&attachment_pointer).await?;

        let date = Utc::now().to_rfc3339();
        let filename = match attachment_pointer.content_type.as_deref() {
            Some("image/jpeg") => format!("signal-{}.jpg", date),
            Some("image/gif") => format!("signal-{}.gif", date),
            Some("image/png") => format!("signal-{}.png", date),
            Some(mimetype) => {
                warn!("unsupported attachment mimetype: {}", mimetype);
                format!("signal-{}", date)
            }
            None => {
                format!("signal-{}", date)
            }
        };

        let filepath = data_dir.join(filename);
        std::fs::write(&filepath, &attachment_data)?;

        Ok(Attachment {
            id: date,
            content_type: attachment_pointer.content_type.unwrap(),
            filename: filepath,
            size: attachment_pointer.size.unwrap(),
        })
    }

    async fn request_contacts_sync(&self) -> anyhow::Result<()> {
        Ok(self.manager.request_contacts_sync().await?)
    }

    fn contact_by_id(&self, id: Uuid) -> anyhow::Result<Option<Contact>> {
        Ok(self.manager.get_contact_by_id(id)?)
    }
}

async fn upload_attachments(
    manager: &presage::Manager<presage::SledConfigStore>,
    attachments: Vec<(AttachmentSpec, Vec<u8>)>,
    data_message: &mut DataMessage,
) {
    match manager.upload_attachments(attachments).await {
        Ok(attachment_pointers) => {
            data_message.attachments = attachment_pointers
                .into_iter()
                .filter_map(|res| {
                    if let Err(e) = res.as_ref() {
                        error!("failed to upload attachment: {}", e);
                    }
                    res.ok()
                })
                .collect();
        }
        Err(e) => {
            error!("failed to upload attachments: {}", e);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub content_type: String,
    pub filename: PathBuf,
    pub size: u32,
}

/// If `db_path` does not exist, it will be created (including parent directories).
fn get_signal_manager(db_path: PathBuf) -> anyhow::Result<Manager> {
    let store = presage::SledConfigStore::new(db_path)?;
    let manager = presage::Manager::with_store(store)?;
    Ok(manager)
}

/// Makes sure that we have a linked device.
///
/// Either,
///
/// 1. links a new device (if no config file found), and writes a new config file with username
///    and phone number, or
/// 2. loads the config file and tries to create the Signal manager from configured Signal database
///    path.
pub async fn ensure_linked_device(relink: bool) -> anyhow::Result<(Manager, Config)> {
    let config = Config::load_installed()?;
    let db_path = config
        .as_ref()
        .map(|c| c.signal_db_path.clone())
        .unwrap_or_else(config::default_signal_db_path);

    let mut manager = get_signal_manager(db_path)?;

    let is_registered = !relink && manager.is_registered();

    if is_registered {
        if let Some(config) = config {
            return Ok((manager, config));
        }
    }

    // link device
    let at_hostname = hostname::get()
        .ok()
        .and_then(|hostname| {
            hostname
                .to_string_lossy()
                .split('.')
                .find(|s| !s.is_empty())
                .map(|s| format!("@{}", s))
        })
        .unwrap_or_default();
    let device_name = format!("gurk{}", at_hostname);
    println!("Linking new device with device name: {}", device_name);
    manager
        .link_secondary_device(SignalServers::Production, device_name.clone())
        .await?;

    // get profile
    let phone_number = manager
        .phone_number()
        .expect("no phone number after device was linked")
        .format()
        .mode(phonenumber::Mode::E164)
        .to_string();
    let profile = manager
        .retrieve_profile()
        .await
        .context("failed to get the user profile")?;
    let name = profile
        .name
        .map(|name| name.given_name)
        .unwrap_or_else(whoami::username);

    let config = if let Some(config) = config {
        // check that config fits the profile
        if config.user.phone_number != phone_number {
            bail!("Wrong phone number in the config. Please adjust it.");
        }
        config
    } else {
        let user = config::User { name, phone_number };
        let config = config::Config::with_user(user);
        config.save_new().context("failed to init config file")?;
        config
    };

    Ok((manager, config))
}

#[cfg(test)]
pub mod test {
    use super::*;

    use std::{cell::RefCell, rc::Rc};

    /// Signal manager mock which does not send any messages.
    pub struct SignalManagerMock {
        user_id: Uuid,
        emoji_replacer: Replacer,
        pub sent_messages: Rc<RefCell<Vec<Message>>>,
    }

    impl SignalManagerMock {
        pub fn new() -> Self {
            Self {
                user_id: Uuid::new_v4(),
                emoji_replacer: Replacer::new(),
                sent_messages: Default::default(),
            }
        }
    }

    #[async_trait(?Send)]
    impl SignalManager for SignalManagerMock {
        fn user_id(&self) -> Uuid {
            self.user_id
        }

        fn send_receipt(&self, _: Uuid, _: Vec<u64>, _: Receipt) {}

        async fn resolve_group(
            &mut self,
            _master_key_bytes: super::GroupMasterKeyBytes,
        ) -> anyhow::Result<super::ResolvedGroup> {
            bail!("mocked signal manager cannot resolve groups");
        }

        fn send_text(
            &self,
            _channel: &crate::app::Channel,
            text: String,
            quote_message: Option<&crate::app::Message>,
            _attachments: Vec<(AttachmentSpec, Vec<u8>)>,
        ) -> Message {
            let message: String = self.emoji_replacer.replace_all(&text).into_owned();
            let timestamp = utc_now_timestamp_msec();
            let quote = quote_message.map(|message| Quote {
                id: Some(message.arrived_at),
                author_uuid: Some(message.from_id.to_string()),
                text: message.message.clone(),
                ..Default::default()
            });
            let quote_message = quote.and_then(Message::from_quote).map(Box::new);
            let message = Message {
                from_id: self.user_id(),
                message: Some(message),
                arrived_at: timestamp,
                quote: quote_message,
                attachments: Default::default(),
                reactions: Default::default(),
                // TODO make sure the message sending procedure did not fail
                receipt: Receipt::Sent,
            };
            self.sent_messages.borrow_mut().push(message.clone());
            println!("sent messages: {:?}", self.sent_messages.borrow());
            message
        }

        fn send_reaction(
            &self,
            _channel: &crate::app::Channel,
            _message: &crate::app::Message,
            _emoji: String,
            _remove: bool,
        ) {
        }

        async fn save_attachment(
            &mut self,
            _attachment_pointer: AttachmentPointer,
        ) -> anyhow::Result<Attachment> {
            bail!("mocked signal manager cannot save attachments");
        }

        async fn resolve_name_from_profile(
            &self,
            _id: Uuid,
            _profile_key: ProfileKey,
        ) -> Option<String> {
            None
        }

        async fn request_contacts_sync(&self) -> anyhow::Result<()> {
            Ok(())
        }

        fn contact_by_id(&self, _id: Uuid) -> anyhow::Result<Option<Contact>> {
            Ok(None)
        }
    }
}
