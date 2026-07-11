-- Revert messages to a global primary key on arrived_at
--
-- Lossy by design: the composite PK allows the same arrived_at in different
-- channels; the global PK cannot. On conflict the first row wins (OR IGNORE).
CREATE TABLE messages_old (
  arrived_at INTEGER PRIMARY KEY NOT NULL,
  channel_id BLOB NOT NULL, -- uuid or group id
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
  expires_at INTEGER
);

INSERT
OR IGNORE INTO messages_old (
  arrived_at,
  channel_id,
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
)
SELECT
  arrived_at,
  channel_id,
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
  messages;

DROP TABLE messages;

ALTER TABLE messages_old
RENAME TO messages;

-- Recreate indexes
CREATE INDEX idx_messages_channel_id ON messages (channel_id);

CREATE INDEX idx_messages_quote ON messages (quote);

CREATE INDEX idx_messages_edit ON messages (edit);

CREATE INDEX idx_messages_expires_at ON messages (expires_at)
WHERE
  expires_at IS NOT NULL;
