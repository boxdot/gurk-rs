use crate::data::ChannelId;
use crate::storage::MessageId;
use tokio::sync::oneshot;

#[derive(Debug)]
pub enum Event {
    SentTextResult {
        message_id: MessageId,
        result: anyhow::Result<()>,
    },
}

/// Commands that can be sent from the API to the App
pub struct ApiCommand {
    pub kind: ApiCommandKind,
    pub response_tx: oneshot::Sender<ApiResponse>,
}

impl std::fmt::Debug for ApiCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiCommand")
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Debug)]
pub enum ApiCommandKind {
    SendMessage {
        channel_id: ChannelId,
        text: String,
    },
}

#[derive(Debug)]
pub enum ApiResponse {
    MessageSent { arrived_at: u64 },
    Error(String),
}
