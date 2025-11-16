use std::fs::OpenOptions;
use std::io::BufWriter;

use base64::prelude::*;
use presage::libsignal_service::protocol::ServiceId;
use presage::libsignal_service::{
    content::{Content, Metadata},
    prelude::DeviceId,
};
use presage::proto;
use prost::Message;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
pub struct ContentBase64 {
    #[serde(with = "MetadataDef")]
    pub metadata: Metadata,
    pub content_proto_base64: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Metadata")]
struct MetadataDef {
    #[serde(with = "service_id")]
    sender: ServiceId,
    #[serde(with = "service_id")]
    destination: ServiceId,
    #[serde(with = "device_id")]
    sender_device: DeviceId,
    timestamp: u64,
    needs_receipt: bool,
    unidentified_sender: bool,
    server_guid: Option<Uuid>,
    was_plaintext: bool,
}

mod service_id {
    use presage::libsignal_service::protocol::ServiceId;
    use serde::{Deserialize, Serialize, Serializer};

    pub fn serialize<S>(value: &ServiceId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.service_id_string().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ServiceId, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        ServiceId::parse_from_service_id_string(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid service id string: {s}")))
    }
}

mod device_id {
    use presage::libsignal_service::protocol::DeviceId;
    use serde::{Deserialize, Serialize, Serializer};

    pub fn serialize<S>(value: &DeviceId, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        u8::from(*value).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DeviceId, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        DeviceId::try_from(value).map_err(serde::de::Error::custom)
    }
}

impl From<Metadata> for MetadataDef {
    fn from(metadata: Metadata) -> Self {
        Self {
            sender: metadata.sender,
            destination: metadata.destination,
            sender_device: metadata.sender_device,
            timestamp: metadata.timestamp,
            needs_receipt: metadata.needs_receipt,
            unidentified_sender: metadata.unidentified_sender,
            server_guid: metadata.server_guid,
            was_plaintext: metadata.was_plaintext,
        }
    }
}

impl From<&Content> for ContentBase64 {
    fn from(content: &Content) -> Self {
        let content_proto_base64 =
            BASE64_STANDARD.encode(content.body.clone().into_proto().encode_to_vec());
        Self {
            metadata: content.metadata.clone(),
            content_proto_base64,
        }
    }
}

impl TryFrom<ContentBase64> for Content {
    type Error = anyhow::Error;

    fn try_from(content: ContentBase64) -> Result<Self, Self::Error> {
        let content_bytes = BASE64_STANDARD.decode(&content.content_proto_base64)?;
        let content_proto = proto::Content::decode(&*content_bytes)?;
        Ok(Self::from_proto(content_proto, content.metadata)?)
    }
}

pub fn dump_raw_message(content: &Content) -> anyhow::Result<()> {
    use std::io::Write;

    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .open("messages.raw.json")?;
    let mut writer = BufWriter::new(f);

    let content = ContentBase64::from(content);

    serde_json::to_writer(&mut writer, &content)?;
    writeln!(writer)?;

    Ok(())
}
