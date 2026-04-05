DROP INDEX idx_messages_expires_at;

ALTER TABLE messages DROP COLUMN expires_at;

ALTER TABLE messages DROP COLUMN expire_timer;

ALTER TABLE channels DROP COLUMN expire_timer;
