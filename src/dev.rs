use std::fs::OpenOptions;
use std::io::BufWriter;

use presage::prelude::proto;
use presage::prelude::{Content, Metadata, ServiceAddress};
use prost::Message;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ContentBase64 {
    #[serde(with = "MetadataDef")]
    pub metadata: Metadata,
    pub content_proto_base64: String,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "Metadata")]
struct MetadataDef {
    sender: ServiceAddress,
    sender_device: u32,
    timestamp: u64,
    needs_receipt: bool,
    unidentified_sender: bool,
}

impl From<Metadata> for MetadataDef {
    fn from(metadata: Metadata) -> Self {
        Self {
            sender: metadata.sender,
            sender_device: metadata.sender_device,
            timestamp: metadata.timestamp,
            needs_receipt: metadata.needs_receipt,
            unidentified_sender: metadata.unidentified_sender,
        }
    }
}

impl From<&Content> for ContentBase64 {
    fn from(content: &Content) -> Self {
        let content_proto_base64 =
            base64::encode(&content.body.clone().into_proto().encode_to_vec());
        Self {
            metadata: content.metadata.clone(),
            content_proto_base64,
        }
    }
}

impl TryFrom<ContentBase64> for Content {
    type Error = anyhow::Error;

    fn try_from(content: ContentBase64) -> Result<Self, Self::Error> {
        let content_bytes = base64::decode(&content.content_proto_base64)?;
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
