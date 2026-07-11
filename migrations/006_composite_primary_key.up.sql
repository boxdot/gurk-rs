REINDEX;

-- Migrate messages to a composite primary key
CREATE TABLE messages_new (
  channel_id BLOB NOT NULL, -- uuid or group id
  arrived_at INTEGER NOT NULL,
  from_id BLOB NOT NULL,
  message TEXT,
  quote INTEGER, -- reference into messages to arrived_at
  receipt BLOB, -- encoded Receipt
  body_ranges BLOB, -- encoded Vec<BodyRange>
  attachments BLOB, -- encoded Vec<Attachment>
  reactions BLOB, -- encoded Vec<(Uuid, String)>
  edit INTEGER,
  edited BOOLEAN NOT NULL DEFAULT FALSE,
  deleted BOOLEAN NOT NULL DEFAULT FALSE,
  expire_timer INTEGER,
  expires_at INTEGER,
  PRIMARY KEY (channel_id, arrived_at),
  FOREIGN KEY (channel_id) REFERENCES channels (id) ON DELETE CASCADE
);

-- We only copy the messages that belong to a channel because of the new
-- foreign key constraint.
INSERT INTO
  messages_new
SELECT
  channel_id,
  arrived_at,
  from_id,
  message,
  quote,
  receipt,
  body_ranges,
  attachments,
  reactions,
  edit,
  edited,
  deleted,
  expire_timer,
  expires_at
FROM
  messages
WHERE
  channel_id IN (
    SELECT
      id
    FROM
      channels
  );

DROP TABLE messages;

ALTER TABLE messages_new
RENAME TO messages;

-- Recreate indexes
CREATE INDEX idx_messages_quote ON messages (quote);

CREATE INDEX idx_messages_edit ON messages (edit);

CREATE INDEX idx_messages_expires_at ON messages (expires_at)
WHERE
  expires_at IS NOT NULL;
