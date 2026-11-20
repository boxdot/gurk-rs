use uuid::Uuid;

use crate::signal::Attachment;
use crate::storage::MessageId;

#[derive(Debug)]
pub enum Event {
    SentTextResult {
        message_id: MessageId,
        result: anyhow::Result<()>,
    },
    AttachmentSaved {
        message_id: MessageId,
        attachment: anyhow::Result<Attachment>,
    },
    ProfileNameResolved {
        user_id: Uuid,
        name: Option<String>,
    },
}
