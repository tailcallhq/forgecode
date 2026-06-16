ALTER TABLE conversations ADD COLUMN source TEXT;

-- Create index for filtering by source
CREATE INDEX idx_conversations_source ON conversations(source);
