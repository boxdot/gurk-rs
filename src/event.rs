use std::path::PathBuf;

use ratatui::layout::Rect;
use ratatui_image::protocol::Protocol;

use crate::storage::MessageId;

pub enum Event {
    SentTextResult {
        message_id: MessageId,
        result: anyhow::Result<()>,
    },
    ImageLoaded {
        path: PathBuf,
        result: Option<(Protocol, Rect)>,
    },
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::SentTextResult { message_id, .. } => f
                .debug_struct("SentTextResult")
                .field("message_id", message_id)
                .finish_non_exhaustive(),
            Event::ImageLoaded { path, result } => f
                .debug_struct("ImageLoaded")
                .field("path", path)
                .field("result", &result.as_ref().map(|(_, rect)| rect))
                .finish(),
        }
    }
}
