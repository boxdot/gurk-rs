-- messages with added field `edit` and foreign key on `channels`
ALTER TABLE messages
ADD COLUMN edit INTEGER;  -- reference to edited message

CREATE INDEX idx_messages_quote
ON messages (quote);

CREATE INDEX idx_messages_edit
ON messages (edit);
