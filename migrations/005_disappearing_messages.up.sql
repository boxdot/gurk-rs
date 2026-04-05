ALTER TABLE channels ADD COLUMN expire_timer INTEGER;

ALTER TABLE messages ADD COLUMN expire_timer INTEGER;

ALTER TABLE messages ADD COLUMN expires_at INTEGER;

CREATE INDEX idx_messages_expires_at ON messages (expires_at)
    WHERE expires_at IS NOT NULL;
