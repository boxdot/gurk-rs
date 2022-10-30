use presage::prelude::proto::{CallMessage, ReceiptMessage, TypingMessage};
use presage::prelude::{Content, ContentBody, DataMessage, Metadata, ServiceAddress, SyncMessage};
use prost::Message;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ContentBase64 {
    #[serde(with = "MetadataDef")]
    pub metadata: Metadata,
    pub body_type: ContentBodyType,
    pub body_proto_base64: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Metadata")]
struct MetadataDef {
    sender: ServiceAddress,
    sender_device: u32,
    timestamp: u64,
    needs_receipt: bool,
}

impl From<Metadata> for MetadataDef {
    fn from(metadata: Metadata) -> Self {
        Self {
            sender: metadata.sender,
            sender_device: metadata.sender_device,
            timestamp: metadata.timestamp,
            needs_receipt: metadata.needs_receipt,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub enum ContentBodyType {
    DataMessage,
    SynchronizeMessage,
    CallMessage,
    ReceiptMessage,
    TypingMessage,
}

impl From<&Content> for ContentBase64 {
    fn from(content: &Content) -> Self {
        use ContentBody::*;
        let (body_type, body_proto_bytes) = match &content.body {
            DataMessage(msg) => (ContentBodyType::DataMessage, msg.encode_to_vec()),
            SynchronizeMessage(msg) => (ContentBodyType::SynchronizeMessage, msg.encode_to_vec()),
            CallMessage(msg) => (ContentBodyType::CallMessage, msg.encode_to_vec()),
            ReceiptMessage(msg) => (ContentBodyType::ReceiptMessage, msg.encode_to_vec()),
            TypingMessage(msg) => (ContentBodyType::TypingMessage, msg.encode_to_vec()),
        };

        let body_proto_base64 = base64::encode(&body_proto_bytes);
        Self {
            metadata: content.metadata.clone(),
            body_type,
            body_proto_base64,
        }
    }
}

impl TryFrom<ContentBase64> for Content {
    type Error = anyhow::Error;

    fn try_from(content: ContentBase64) -> Result<Self, Self::Error> {
        let body_bytes = base64::decode(&content.body_proto_base64)?;
        let buf = body_bytes.as_slice();
        let body = match content.body_type {
            ContentBodyType::DataMessage => DataMessage::decode(buf)?.into(),
            ContentBodyType::SynchronizeMessage => SyncMessage::decode(buf)?.into(),
            ContentBodyType::CallMessage => CallMessage::decode(buf)?.into(),
            ContentBodyType::ReceiptMessage => ReceiptMessage::decode(buf)?.into(),
            ContentBodyType::TypingMessage => TypingMessage::decode(buf)?.into(),
        };
        Ok(Self {
            metadata: content.metadata,
            body,
        })
    }
}
