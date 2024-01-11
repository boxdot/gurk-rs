DROP INDEX idx_messages_edit;

DROP INDEX idx_messages_quote;

ALTER TABLE messages
DROP COLUMN edit;
