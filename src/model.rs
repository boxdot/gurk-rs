use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub envelope: Envelope,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub source: String,
    pub source_device: u8,
    // pub relay: Option<?>,
    pub timestamp: u64,
    pub message: Option<String>,
    pub is_receipt: bool,
    pub is_read: Option<bool>,
    pub is_delivery: Option<bool>,
    pub data_message: Option<DataMessage>,
    // sync_message
    // call_message
    // receipt_message
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMessage {
    pub timestamp: u64,
    pub message: String,
    pub expires_in_seconds: u64,
    // attachments,
    pub group_info: Option<GroupInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    pub group_id: String,
    // members
    pub name: Option<String>,
    pub members: Option<Vec<String>>,
    // type
}
