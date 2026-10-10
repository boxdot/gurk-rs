mod hypermedia;
mod routes;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use axum::{
    Router,
    extract::State,
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use tokio::sync::{RwLock, broadcast, mpsc};

use crate::config::Config;
use crate::event::ApiCommand;
use crate::storage::Storage;

/// Event types that can be broadcast to SSE clients
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApiEvent {
    MessageReceived {
        chat_id: String,
        message_id: String,
        arrived_at: u64,
    },
    MessageSent {
        chat_id: String,
        message_id: String,
        arrived_at: u64,
    },
    TypingStarted {
        chat_id: String,
        user_id: String,
    },
    TypingStopped {
        chat_id: String,
        user_id: String,
    },
}

impl ApiEvent {
    /// Returns the chat_id associated with this event.
    pub fn chat_id(&self) -> &str {
        match self {
            Self::MessageReceived { chat_id, .. }
            | Self::MessageSent { chat_id, .. }
            | Self::TypingStarted { chat_id, .. }
            | Self::TypingStopped { chat_id, .. } => chat_id,
        }
    }

    /// Returns the event type as a snake_case string.
    pub fn event_type(&self) -> &str {
        match self {
            Self::MessageReceived { .. } => "message_received",
            Self::MessageSent { .. } => "message_sent",
            Self::TypingStarted { .. } => "typing_started",
            Self::TypingStopped { .. } => "typing_stopped",
        }
    }
}

pub use hypermedia::{Action, HypermediaResource, Links};

/// API token with secure memory handling.
///
/// The inner string is zeroized on drop to prevent secrets from lingering in memory.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct ApiToken(String);

impl ApiToken {
    /// Create a new API token from a string.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// Get the token bytes for constant-time comparison.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for ApiToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiToken(<redacted>)")
    }
}

/// Shared state for the API server
pub struct ApiState<S> {
    pub storage: Arc<RwLock<S>>,
    pub command_tx: mpsc::Sender<ApiCommand>,
    pub event_tx: broadcast::Sender<ApiEvent>,
    pub token: ApiToken,
    pub version: &'static str,
    pub data_dir: PathBuf,
}

/// Start the API server if enabled in config
/// Returns an event sender for broadcasting events to SSE clients
pub async fn start_server<S>(
    config: &Config,
    storage: Arc<RwLock<S>>,
    command_tx: mpsc::Sender<ApiCommand>,
) -> anyhow::Result<Option<broadcast::Sender<ApiEvent>>>
where
    S: Storage + Send + Sync + 'static,
{
    if !config.api.enabled {
        return Ok(None);
    }

    let token = ApiToken::new(config.api_token()?);
    let bind = config.api.bind.clone();
    let (event_tx, _) = broadcast::channel(256);

    let state = Arc::new(ApiState {
        storage,
        command_tx,
        event_tx: event_tx.clone(),
        token,
        version: env!("CARGO_PKG_VERSION"),
        data_dir: config.data_dir.clone(),
    });

    let app = Router::new()
        .merge(routes::router())
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("API server failed to bind to {}: {} (continuing without API)", bind, e);
            return Ok(None);
        }
    };
    tracing::info!("API server listening on {}", bind);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("API server error: {}", e);
        }
    });

    Ok(Some(event_tx))
}

/// Normalize a URL path by resolving `.` and `..` segments.
/// This prevents path traversal bypasses like `/v1/schemas/../chats`.
fn normalize_path(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

async fn auth_middleware<S>(
    State(state): State<Arc<ApiState<S>>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode>
where
    S: Storage + Send + Sync + 'static,
{
    // Normalize the path to prevent traversal bypasses like /v1/schemas/../chats
    let path = normalize_path(request.uri().path());

    // Allow unauthenticated access to service root and schemas
    if path == "/" || path == "/v1/schemas" || path.starts_with("/v1/schemas/") {
        return Ok(next.run(request).await);
    }

    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(auth) if auth.starts_with("Bearer ") => {
            let provided_token = &auth[7..];
            let eq = provided_token.as_bytes().ct_eq(state.token.as_bytes());
            if eq.into() {
                Ok(next.run(request).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
