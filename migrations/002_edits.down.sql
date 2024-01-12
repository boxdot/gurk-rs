DROP INDEX idx_messages_edit;

DROP INDEX idx_messages_quote;

ALTER TABLE messages
DROP COLUMN edited;

ALTER TABLE messages
DROP COLUMN edit;
