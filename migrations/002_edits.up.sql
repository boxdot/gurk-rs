-- edit points to the original edited message
--
-- for a chain of edits original, edit1, edit2, ... each edit points to the arrived_at field of
-- the message original
ALTER TABLE messages
ADD COLUMN edit INTEGER;

ALTER TABLE messages
ADD COLUMN edited BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX idx_messages_quote
ON messages (quote);

CREATE INDEX idx_messages_edit
ON messages (edit);
