use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, sse::{Event, Sse}},
    routing::get,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};

use crate::data::{ChannelId, Message};
use crate::event::{ApiCommand, ApiCommandKind, ApiResponse};
use crate::storage::Storage;

use super::hypermedia::{Action, HypermediaCollection, HypermediaResource};
use super::ApiState;

// --- Error Response ---

#[derive(Serialize)]
struct ApiError {
    error: String,
}

impl ApiError {
    fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<Self>) {
        (StatusCode::BAD_REQUEST, Json(Self { error: msg.into() }))
    }

    fn not_found(msg: impl Into<String>) -> (StatusCode, Json<Self>) {
        (StatusCode::NOT_FOUND, Json(Self { error: msg.into() }))
    }

    fn forbidden(msg: impl Into<String>) -> (StatusCode, Json<Self>) {
        (StatusCode::FORBIDDEN, Json(Self { error: msg.into() }))
    }

    fn internal(msg: impl Into<String>) -> (StatusCode, Json<Self>) {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(Self { error: msg.into() }))
    }
}

type ApiResult<T> = Result<T, (StatusCode, Json<ApiError>)>;

fn is_false(b: &bool) -> bool {
    !*b
}

pub fn router<S>() -> Router<Arc<ApiState<S>>>
where
    S: Storage + Send + Sync + 'static,
{
    Router::new()
        .route("/", get(service_root))
        .route("/v1/chats", get(list_chats))
        .route("/v1/chats/{id}", get(get_chat))
        .route("/v1/chats/{id}/messages", get(list_messages).post(send_message))
        .route("/v1/events", get(events_stream))
        .route("/v1/attachments/{id}", get(get_attachment))
        .route("/v1/contacts", get(list_contacts))
        .route("/v1/contacts/{id}", get(get_contact))
        .route("/v1/names", get(list_names))
        .route("/v1/names/{id}", get(get_name))
        .route("/v1/schemas", get(list_schemas))
        .route("/v1/schemas/{name}", get(get_schema))
}

// --- Service Root ---

#[derive(Serialize)]
struct ServiceRoot {
    version: &'static str,
    connected: bool,
}

async fn service_root<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
) -> Json<HypermediaResource<ServiceRoot>> {
    let mut resource = HypermediaResource::new(
        "ServiceRoot",
        "/",
        ServiceRoot {
            version: state.version,
            connected: true, // ponytail: always true for now, add real check later
        },
    );
    resource.links.insert("chats", "/v1/chats");
    resource.links.insert("contacts", "/v1/contacts");
    resource.links.insert("events", "/v1/events");
    resource.links.insert("names", "/v1/names");
    resource.links.insert("schemas", "/v1/schemas");
    Json(resource)
}

// --- SSE Events ---

#[derive(Deserialize, Default)]
struct EventsQuery {
    chat_id: Option<String>,
    types: Option<String>, // comma-separated: message_received,typing_started
}

async fn events_stream<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
    Query(query): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let chat_filter = query.chat_id;
    let type_filter: Option<Vec<String>> = query
        .types
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect());

    let stream = BroadcastStream::new(rx).filter_map(move |result| {
        match result {
            Ok(event) => {
                // Filter by chat_id if specified
                if let Some(ref filter_chat) = chat_filter {
                    if event.chat_id() != filter_chat {
                        return None;
                    }
                }

                // Filter by event type if specified
                if let Some(ref allowed_types) = type_filter {
                    if !allowed_types.iter().any(|t| t == event.event_type()) {
                        return None;
                    }
                }

                let json = match serde_json::to_string(&event) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "failed to serialize SSE event");
                        return None;
                    }
                };
                Some(Ok(Event::default().data(json)))
            }
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::warn!(dropped = n, "SSE client lagged, events dropped");
                None
            }
        }
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("ping"),
    )
}

// --- Attachments ---

#[derive(Deserialize)]
struct AttachmentQuery {
    chat_id: String,
    arrived_at: u64,
}

async fn get_attachment<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
    Path(attachment_id): Path<String>,
    Query(query): Query<AttachmentQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ApiError>)> {
    use axum::body::Body;
    use axum::http::header;

    let channel_id = parse_channel_id(&query.chat_id)
        .ok_or_else(|| ApiError::bad_request("invalid chat id format"))?;

    let storage = state.storage.read().await;
    // Query for message AT this timestamp (before is exclusive, so +1)
    let message = storage
        .messages_before(channel_id, query.arrived_at + 1, 1)
        .into_iter()
        .find(|m| m.arrived_at == query.arrived_at)
        .ok_or_else(|| ApiError::not_found("message not found"))?;

    let attachment = message
        .attachments
        .iter()
        .find(|a| a.id == attachment_id)
        .ok_or_else(|| ApiError::not_found("attachment not found"))?;

    // Validate attachment path is within the allowed files directory
    let files_dir = state.data_dir.join("files");
    let canonical_files_dir = files_dir.canonicalize().map_err(|e| {
        tracing::warn!(path = ?files_dir, error = %e, "failed to canonicalize files directory");
        ApiError::not_found("attachment file not found")
    })?;
    let canonical_attachment = attachment.filename.canonicalize().map_err(|e| {
        tracing::warn!(path = ?attachment.filename, error = %e, "failed to canonicalize attachment path");
        ApiError::not_found("attachment file not found")
    })?;

    if !canonical_attachment.starts_with(&canonical_files_dir) {
        return Err(ApiError::forbidden("attachment path not allowed"));
    }

    let file_bytes = tokio::fs::read(&attachment.filename).await.map_err(|e| {
        tracing::warn!(path = ?attachment.filename, error = %e, "failed to read attachment file");
        ApiError::not_found("attachment file not found")
    })?;

    let content_type = attachment.content_type.parse::<mime_guess::mime::Mime>()
        .unwrap_or(mime_guess::mime::APPLICATION_OCTET_STREAM);

    Ok((
        [(header::CONTENT_TYPE, content_type.to_string())],
        Body::from(file_bytes),
    ).into_response())
}

// --- Contacts ---

#[derive(Serialize)]
struct ContactSummary {
    id: String,
    name: String,
}

async fn list_contacts<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
) -> Json<HypermediaCollection<HypermediaResource<ContactSummary>>> {
    let storage = state.storage.read().await;
    let channels = storage.channels();

    // Contacts are 1:1 chats (User channels, not Group)
    let items: Vec<_> = channels
        .into_iter()
        .filter_map(|ch| {
            if let ChannelId::User(uuid) = ch.id {
                let id = format!("user-{}", uuid);
                let href = format!("/v1/contacts/{}", id);
                Some(
                    HypermediaResource::new(
                        "Contact",
                        &href,
                        ContactSummary {
                            id: id.clone(),
                            name: ch.name.clone(),
                        },
                    )
                    .with_link("chat", format!("/v1/chats/{}", id)),
                )
            } else {
                None
            }
        })
        .collect();

    Json(
        HypermediaCollection::new("ContactCollection", "/v1/contacts", items)
            .with_link("schema", "/v1/schemas/Contact"),
    )
}

async fn get_contact<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> ApiResult<Json<HypermediaResource<ContactSummary>>> {
    let channel_id = parse_channel_id(&id)
        .ok_or_else(|| ApiError::bad_request("invalid contact id format"))?;

    // Must be a user channel (not group)
    if !matches!(channel_id, ChannelId::User(_)) {
        return Err(ApiError::bad_request("contact id must be a user, not a group"));
    }

    let storage = state.storage.read().await;
    let channel = storage
        .channel(channel_id)
        .ok_or_else(|| ApiError::not_found("contact not found"))?;

    let href = format!("/v1/contacts/{}", id);
    let resource = HypermediaResource::new(
        "Contact",
        &href,
        ContactSummary {
            id: id.clone(),
            name: channel.name.clone(),
        },
    )
    .with_link("chat", format!("/v1/chats/{}", id))
    .with_link("schema", "/v1/schemas/Contact");

    Ok(Json(resource))
}

// --- Names ---

#[derive(Serialize)]
struct NameEntry {
    id: String,
    name: String,
}

async fn list_names<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
) -> Json<HypermediaCollection<HypermediaResource<NameEntry>>> {
    let storage = state.storage.read().await;

    let items: Vec<_> = storage
        .names()
        .map(|(uuid, name)| {
            let id = uuid.to_string();
            let href = format!("/v1/names/{}", id);
            HypermediaResource::new(
                "Name",
                &href,
                NameEntry {
                    id: id.clone(),
                    name,
                },
            )
        })
        .collect();

    Json(HypermediaCollection::new("NameCollection", "/v1/names", items))
}

async fn get_name<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> ApiResult<Json<HypermediaResource<NameEntry>>> {
    let uuid: uuid::Uuid = id
        .parse()
        .map_err(|_| ApiError::bad_request("invalid uuid format"))?;

    let storage = state.storage.read().await;
    let name = storage
        .name(uuid)
        .ok_or_else(|| ApiError::not_found("name not found"))?;

    let href = format!("/v1/names/{}", id);
    let resource = HypermediaResource::new(
        "Name",
        &href,
        NameEntry {
            id: id.clone(),
            name,
        },
    );

    Ok(Json(resource))
}

// --- Chats ---

#[derive(Serialize)]
struct ChatSummary {
    id: String,
    name: String,
    group: bool,
    unread: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    members: Option<Vec<MemberSummary>>,
}

#[derive(Serialize)]
struct MemberSummary {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

fn build_members(
    group_data: Option<&crate::data::GroupData>,
    names: &std::collections::HashMap<uuid::Uuid, String>,
) -> Option<Vec<MemberSummary>> {
    group_data.map(|gd| {
        gd.members
            .iter()
            .map(|uuid| MemberSummary {
                id: uuid.to_string(),
                name: names.get(uuid).cloned(),
            })
            .collect()
    })
}

async fn list_chats<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
) -> Json<HypermediaCollection<HypermediaResource<ChatSummary>>> {
    let storage = state.storage.read().await;
    let channels = storage.channels();
    let names: std::collections::HashMap<uuid::Uuid, String> = storage.names().collect();

    let items: Vec<_> = channels
        .into_iter()
        .map(|ch| {
            let id = channel_id_to_string(&ch.id);
            let href = format!("/v1/chats/{}", id);
            let members = build_members(ch.group_data.as_ref(), &names);
            HypermediaResource::new(
                "Chat",
                &href,
                ChatSummary {
                    id: id.clone(),
                    name: ch.name.clone(),
                    group: matches!(ch.id, ChannelId::Group(_)),
                    unread: ch.unread_messages,
                    members,
                },
            )
            .with_link("messages", format!("{}/messages", href))
            .with_action("send", Action::post(format!("{}/messages", href)))
        })
        .collect();

    Json(
        HypermediaCollection::new("ChatCollection", "/v1/chats", items)
            .with_link("schema", "/v1/schemas/Chat"),
    )
}

async fn get_chat<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> ApiResult<Json<HypermediaResource<ChatSummary>>> {
    let channel_id = parse_channel_id(&id)
        .ok_or_else(|| ApiError::bad_request("invalid chat id format"))?;

    let storage = state.storage.read().await;
    let channel = storage
        .channel(channel_id)
        .ok_or_else(|| ApiError::not_found("chat not found"))?;

    let names: std::collections::HashMap<uuid::Uuid, String> = storage.names().collect();
    let members = build_members(channel.group_data.as_ref(), &names);

    let href = format!("/v1/chats/{}", id);
    let resource = HypermediaResource::new(
        "Chat",
        &href,
        ChatSummary {
            id: id.clone(),
            name: channel.name.clone(),
            group: matches!(channel.id, ChannelId::Group(_)),
            unread: channel.unread_messages,
            members,
        },
    )
    .with_link("messages", format!("{}/messages", href))
    .with_link("schema", "/v1/schemas/Chat")
    .with_action(
        "send",
        Action::post(format!("{}/messages", href)).with_schema("/v1/schemas/SendMessage"),
    );

    Ok(Json(resource))
}

// --- Messages ---

#[derive(Deserialize)]
struct MessagesQuery {
    limit: Option<usize>,
    before: Option<u64>,
}

#[derive(Serialize)]
struct MessageSummary {
    id: String,
    text: Option<String>,
    from_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_name: Option<String>,
    arrived_at: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<AttachmentSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    link_previews: Vec<LinkPreviewSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reactions: Vec<ReactionSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quote: Option<Box<QuoteSummary>>,
    #[serde(skip_serializing_if = "is_false")]
    edited: bool,
    #[serde(skip_serializing_if = "is_false")]
    deleted: bool,
}

#[derive(Serialize)]
struct AttachmentSummary {
    id: String,
    content_type: String,
    size: u32,
}

#[derive(Serialize)]
struct LinkPreviewSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Serialize)]
struct ReactionSummary {
    user_id: String,
    emoji: String,
}

#[derive(Serialize)]
struct QuoteSummary {
    from_id: String,
    arrived_at: u64,
    text: Option<String>,
}

async fn list_messages<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
    Path(chat_id): Path<String>,
    Query(query): Query<MessagesQuery>,
) -> ApiResult<Json<HypermediaCollection<HypermediaResource<MessageSummary>>>> {
    let channel_id = parse_channel_id(&chat_id)
        .ok_or_else(|| ApiError::bad_request("invalid chat id format"))?;
    let limit = query.limit.unwrap_or(50).min(200);

    let storage = state.storage.read().await;

    // Build names lookup map
    let names: std::collections::HashMap<uuid::Uuid, String> = storage.names().collect();

    let messages = match query.before {
        Some(before) => storage.messages_before(channel_id, before, limit),
        None => storage.messages_tail(channel_id, limit),
    };

    let items: Vec<_> = messages
        .into_iter()
        .map(|msg| {
            let msg_id = format!("{}-{}", chat_id, msg.arrived_at);
            HypermediaResource::new(
                "Message",
                format!("/v1/messages/{}", msg_id),
                message_to_summary(&chat_id, msg, &names),
            )
            .with_link("chat", format!("/v1/chats/{}", chat_id))
        })
        .collect();

    let self_href = format!("/v1/chats/{}/messages", chat_id);
    // Only include next link if we returned a full page (more messages may exist)
    let pagination_next = if items.len() == limit {
        items.first().map(|m| format!("{}?before={}&limit={}", self_href, m.data.arrived_at, limit))
    } else {
        None
    };

    Ok(Json(
        HypermediaCollection::new("MessageCollection", &self_href, items)
            .with_link("chat", format!("/v1/chats/{}", chat_id))
            .with_pagination(pagination_next, None),
    ))
}

// --- Send Message ---

#[derive(Deserialize)]
struct SendMessageRequest {
    text: String,
}

#[derive(Serialize)]
struct SendMessageResponse {
    id: String,
    arrived_at: u64,
}

async fn send_message<S: Storage + Send + Sync>(
    State(state): State<Arc<ApiState<S>>>,
    Path(chat_id): Path<String>,
    Json(body): Json<SendMessageRequest>,
) -> ApiResult<Json<HypermediaResource<SendMessageResponse>>> {
    // Reject empty or whitespace-only message text
    if body.text.trim().is_empty() {
        return Err(ApiError::bad_request("message text cannot be empty"));
    }

    let channel_id = parse_channel_id(&chat_id)
        .ok_or_else(|| ApiError::bad_request("invalid chat id format"))?;

    let (response_tx, response_rx) = oneshot::channel();

    let command = ApiCommand {
        kind: ApiCommandKind::SendMessage {
            channel_id,
            text: body.text,
        },
        response_tx,
    };

    state
        .command_tx
        .send(command)
        .await
        .map_err(|_| ApiError::internal("failed to send command"))?;

    let response = response_rx
        .await
        .map_err(|_| ApiError::internal("failed to receive response"))?;

    match response {
        ApiResponse::MessageSent { arrived_at } => {
            let msg_id = format!("{}-{}", chat_id, arrived_at);
            let resource = HypermediaResource::new(
                "Message",
                format!("/v1/messages/{}", msg_id),
                SendMessageResponse {
                    id: msg_id,
                    arrived_at,
                },
            )
            .with_link("chat", format!("/v1/chats/{}", chat_id));
            Ok(Json(resource))
        }
        ApiResponse::Error(e) => Err(ApiError::internal(e)),
    }
}

// --- Schemas ---

async fn list_schemas<S: Storage + Send + Sync>(
    State(_state): State<Arc<ApiState<S>>>,
) -> Json<HypermediaCollection<SchemaRef>> {
    let schemas = vec![
        SchemaRef {
            name: "Chat",
            id: "/v1/schemas/Chat".into(),
        },
        SchemaRef {
            name: "Contact",
            id: "/v1/schemas/Contact".into(),
        },
        SchemaRef {
            name: "Message",
            id: "/v1/schemas/Message".into(),
        },
        SchemaRef {
            name: "SendMessage",
            id: "/v1/schemas/SendMessage".into(),
        },
    ];
    Json(HypermediaCollection::new(
        "SchemaCollection",
        "/v1/schemas",
        schemas,
    ))
}

#[derive(Serialize)]
struct SchemaRef {
    name: &'static str,
    #[serde(rename = "@id")]
    id: String,
}

async fn get_schema<S: Storage + Send + Sync>(
    State(_state): State<Arc<ApiState<S>>>,
    Path(name): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    // ponytail: hardcoded schemas for now, use schemars derive later
    let schema = match name.as_str() {
        "Chat" => serde_json::json!({
            "$id": "/v1/schemas/Chat",
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "name": {"type": "string"},
                "group": {"type": "boolean"},
                "unread": {"type": "integer"}
            }
        }),
        "Contact" => serde_json::json!({
            "$id": "/v1/schemas/Contact",
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "name": {"type": "string"}
            }
        }),
        "Message" => serde_json::json!({
            "$id": "/v1/schemas/Message",
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "text": {"type": ["string", "null"]},
                "from_id": {"type": "string"},
                "arrived_at": {"type": "integer"}
            }
        }),
        "SendMessage" => serde_json::json!({
            "$id": "/v1/schemas/SendMessage",
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": {"type": "string"}
            }
        }),
        _ => return Err(ApiError::not_found(format!("schema '{}' not found", name))),
    };
    Ok(Json(schema))
}

// --- Helpers ---

fn message_to_summary(
    chat_id: &str,
    msg: Message,
    names: &std::collections::HashMap<uuid::Uuid, String>,
) -> MessageSummary {
    let msg_id = format!("{}-{}", chat_id, msg.arrived_at);

    let attachments: Vec<_> = msg
        .attachments
        .iter()
        .map(|a| AttachmentSummary {
            id: a.id.clone(),
            content_type: a.content_type.clone(),
            size: a.size,
        })
        .collect();

    let link_previews: Vec<_> = msg
        .link_previews
        .iter()
        .map(|lp| LinkPreviewSummary {
            url: lp.url.clone(),
            title: lp.title.clone(),
            description: lp.description.clone(),
        })
        .collect();

    let reactions: Vec<_> = msg
        .reactions
        .iter()
        .map(|(user_id, emoji)| ReactionSummary {
            user_id: user_id.to_string(),
            emoji: emoji.clone(),
        })
        .collect();

    let quote = msg.quote.map(|q| {
        Box::new(QuoteSummary {
            from_id: q.from_id.to_string(),
            arrived_at: q.arrived_at,
            text: q.message.clone(),
        })
    });

    MessageSummary {
        id: msg_id,
        text: msg.message,
        from_id: msg.from_id.to_string(),
        from_name: names.get(&msg.from_id).cloned(),
        arrived_at: msg.arrived_at,
        attachments,
        link_previews,
        reactions,
        quote,
        edited: msg.edited,
        deleted: msg.deleted,
    }
}

fn channel_id_to_string(id: &ChannelId) -> String {
    match id {
        ChannelId::User(uuid) => format!("user-{}", uuid),
        ChannelId::Group(bytes) => format!("group-{}", hex::encode(bytes)),
    }
}

fn parse_channel_id(s: &str) -> Option<ChannelId> {
    if let Some(uuid_str) = s.strip_prefix("user-") {
        uuid_str.parse().ok().map(ChannelId::User)
    } else if let Some(hex_str) = s.strip_prefix("group-") {
        let bytes = hex::decode(hex_str).ok()?;
        let arr: [u8; 32] = bytes.try_into().ok()?;
        Some(ChannelId::Group(arr))
    } else {
        None
    }
}
