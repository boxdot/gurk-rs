# Gurk REST API

A hypermedia REST API for programmatic access to gurk's messaging functionality.

## Configuration

Enable the API in `gurk.toml`:

```toml
[api]
enabled = true
bind = "127.0.0.1:23374"  # optional, this is the default
```

## Authentication

All endpoints except `/` and `/v1/schemas/*` require Bearer token authentication.

The token is stored in `<data_dir>/api_token` and generated on first run.

```bash
curl -H "Authorization: Bearer $(cat ~/.local/share/gurk/api_token)" \
     http://127.0.0.1:23374/v1/chats
```

## Hypermedia Format

All responses use a hypermedia format with `@type`, `@id`, `@links`, and `@actions`:

```json
{
  "@type": "Chat",
  "@id": "/v1/chats/user-abc123",
  "@links": {
    "messages": "/v1/chats/user-abc123/messages",
    "schema": "/v1/schemas/Chat"
  },
  "@actions": {
    "send": {"method": "POST", "href": "/v1/chats/user-abc123/messages"}
  },
  "id": "user-abc123",
  "name": "Alice",
  "group": false,
  "unread": 2
}
```

## Endpoints

### Service Root

```
GET /
```

Returns links to all available resources.

---

### Chats

```
GET /v1/chats
```

List all chats (1:1 and groups).

```
GET /v1/chats/{id}
```

Get a single chat. Group chats include `members` array.

**Chat ID format:**
- 1:1 chats: `user-{uuid}`
- Groups: `group-{32-byte-hex}`

**Response fields:**
- `id` - chat identifier
- `name` - display name
- `group` - boolean
- `unread` - unread message count
- `members` - (groups only) array of `{id, name}`

---

### Messages

```
GET /v1/chats/{id}/messages?limit=50&before={timestamp}
```

List messages in a chat, newest first.

**Query params:**
- `limit` - max messages to return (default: 50, max: 200)
- `before` - pagination: get messages before this timestamp

**Response fields:**
- `id` - message identifier (`{chat_id}-{arrived_at}`)
- `text` - message text (null for media-only)
- `from_id` - sender UUID
- `from_name` - sender display name (if known)
- `arrived_at` - timestamp (milliseconds)
- `attachments` - array of `{id, content_type, size}`
- `link_previews` - array of `{url, title, description}`
- `reactions` - array of `{user_id, emoji}`
- `quote` - replied-to message `{from_id, arrived_at, text}`
- `edited` - boolean
- `deleted` - boolean

```
POST /v1/chats/{id}/messages
Content-Type: application/json

{"text": "Hello!"}
```

Send a message to a chat.

---

### Contacts

```
GET /v1/contacts
```

List all contacts (1:1 chats only, not groups).

```
GET /v1/contacts/{id}
```

Get a single contact.

---

### Names

```
GET /v1/names
```

List all known UUID-to-name mappings.

```
GET /v1/names/{uuid}
```

Look up a name by UUID.

---

### Attachments

```
GET /v1/attachments/{id}?chat_id={chat_id}&arrived_at={timestamp}
```

Download an attachment file.

**Query params (required):**
- `chat_id` - the chat containing the message
- `arrived_at` - the message timestamp

Returns the file with appropriate `Content-Type` header.

---

### Events (SSE)

```
GET /v1/events?chat_id={id}&types={types}
```

Server-Sent Events stream for real-time updates.

**Query params (optional):**
- `chat_id` - filter to events for a specific chat
- `types` - comma-separated event types to receive

**Event types:**
- `message_received` - incoming message
- `message_sent` - outgoing message (via API)
- `typing_started` - user started typing
- `typing_stopped` - user stopped typing

**Example:**
```bash
curl -N -H "Authorization: Bearer $TOKEN" \
     "http://127.0.0.1:23374/v1/events?types=message_received"
```

**Event format:**
```json
{"type":"message_received","chat_id":"user-abc","message_id":"user-abc-1696869600000","arrived_at":1696869600000}
```

---

### Schemas

```
GET /v1/schemas
```

List available JSON schemas.

```
GET /v1/schemas/{name}
```

Get a specific schema (Chat, Contact, Message, SendMessage).

## Error Responses

- `400 Bad Request` - invalid parameters
- `401 Unauthorized` - missing or invalid token
- `404 Not Found` - resource not found
- `500 Internal Server Error` - server error

## Notes

- This is a **linked device** API. Contacts are synced from Signal, not writable.
- Voice/video calls are not supported (gurk is text-only).
- The API binds to localhost by default for security.
