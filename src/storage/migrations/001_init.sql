CREATE TABLE channels(
    id BLOB PRIMARY KEY NOT NULL, -- uuid or group id
    name TEXT NOT NULL,
    group_master_key BLOB,
    group_revision INTEGER, -- u32
    group_members BLOB -- encoded Vec<Uuid>
);

CREATE TABLE messages(
    arrived_at INTEGER PRIMARY KEY NOT NULL,
    channel_id BLOB NOT NULL,  -- uuid or group id
    from_id BLOB NOT NULL,
    message TEXT,
    quote INTEGER, -- reference into messages to arrived_at
    receipt BLOB, -- encoded Receipt
    body_ranges BLOB, -- encoded Vec<BodyRange>
    attachments BLOB, -- encoded Vec<Attachment>
    reactions BLOB -- encoded Vec<(Uuid, String)>
);

CREATE INDEX idx_messages_channel_id
ON messages(channel_id);
