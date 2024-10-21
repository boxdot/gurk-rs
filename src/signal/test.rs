use std::pin::Pin;
use std::{cell::RefCell, rc::Rc};

use async_trait::async_trait;
use presage::libsignal_service::content::Content;
use presage::libsignal_service::models::Contact;
use presage::libsignal_service::prelude::AttachmentIdentifier;
use presage::libsignal_service::sender::AttachmentSpec;
use presage::model::groups::Group;
use presage::proto::data_message::Quote;
use presage::proto::AttachmentPointer;
use tokio::sync::oneshot;
use tokio_stream::Stream;
use uuid::Uuid;

use crate::data::{Channel, GroupData, Message};
use crate::receipt::Receipt;
use crate::util::utc_now_timestamp_msec;

use super::{Attachment, GroupMasterKeyBytes, ProfileKeyBytes, ResolvedGroup, SignalManager};

/// Signal manager mock which does not send any messages.
pub struct SignalManagerMock {
    user_id: Uuid,
    pub sent_messages: Rc<RefCell<Vec<Message>>>,
}

impl SignalManagerMock {
    pub fn new() -> Self {
        Self {
            user_id: Uuid::nil(),
            sent_messages: Default::default(),
        }
    }
}

impl Default for SignalManagerMock {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl SignalManager for SignalManagerMock {
    fn user_id(&self) -> Uuid {
        self.user_id
    }

    async fn resolve_group(
        &mut self,
        master_key_bytes: super::GroupMasterKeyBytes,
    ) -> anyhow::Result<ResolvedGroup> {
        Ok(ResolvedGroup {
            name: "some_group".to_string(),
            group_data: GroupData {
                master_key_bytes,
                members: Default::default(),
                revision: 0,
            },
            profile_keys: Default::default(),
        })
    }

    async fn save_attachment(
        &mut self,
        attachment_pointer: AttachmentPointer,
    ) -> anyhow::Result<Attachment> {
        let id = match attachment_pointer.attachment_identifier.unwrap() {
            AttachmentIdentifier::CdnId(id) => id.to_string(),
            AttachmentIdentifier::CdnKey(id) => id,
        };
        Ok(Attachment {
            id,
            content_type: attachment_pointer.content_type.unwrap(),
            filename: "somefile".to_string().into(),
            size: attachment_pointer.size.unwrap(),
        })
    }

    fn send_receipt(&self, _: Uuid, _: Vec<u64>, _: Receipt) {}

    fn send_text(
        &self,
        _channel: &Channel,
        text: String,
        quote_message: Option<&Message>,
        _edit_message_timestamp: Option<u64>,
        _attachments: Vec<(AttachmentSpec, Vec<u8>)>,
    ) -> (Message, oneshot::Receiver<anyhow::Result<()>>) {
        let message: String = crate::emoji::replace_shortcodes(&text).into_owned();
        let timestamp = utc_now_timestamp_msec();
        let quote = quote_message.map(|message| Quote {
            id: Some(message.arrived_at),
            author_aci: Some(message.from_id.to_string()),
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
            receipt: Receipt::Sent,
            body_ranges: Default::default(),
            send_failed: Default::default(),
            edit: Default::default(),
            edited: Default::default(),
        };
        self.sent_messages.borrow_mut().push(message.clone());
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Ok(()));
        (message, rx)
    }

    fn send_reaction(&self, _channel: &Channel, _message: &Message, _emoji: String, _remove: bool) {
    }

    async fn resolve_profile_name(
        &mut self,
        _id: Uuid,
        _profile_key: ProfileKeyBytes,
    ) -> Option<String> {
        None
    }

    async fn request_contacts_sync(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn profile_name(&self, _id: Uuid) -> Option<String> {
        None
    }

    fn contact(&self, _id: Uuid) -> Option<Contact> {
        None
    }

    async fn receive_messages(&mut self) -> anyhow::Result<Pin<Box<dyn Stream<Item = Content>>>> {
        Ok(Box::pin(tokio_stream::empty()))
    }

    fn clone_boxed(&self) -> Box<dyn SignalManager> {
        Box::new(Self {
            user_id: self.user_id,
            sent_messages: self.sent_messages.clone(),
        })
    }

    fn contacts(&self) -> Box<dyn Iterator<Item = Contact>> {
        Box::new(std::iter::empty())
    }

    fn groups(&self) -> Box<dyn Iterator<Item = (GroupMasterKeyBytes, Group)>> {
        Box::new(std::iter::empty())
    }
}
