use crate::storage::MessageId;

#[derive(Debug)]
pub enum Event {
    SentTextResult {
        message_id: MessageId,
        result: anyhow::Result<()>,
    },
}
