-- ADR-103: Intent-gated semantic pruning migration
-- Adds intent state machine columns to track conversation extraction lifecycle
--
-- State machine: pending → extracting → extracted → verified → pruned
-- A conversation can only transition to pruned if intent_state = 'verified'

ALTER TABLE conversations ADD COLUMN intent_state TEXT NOT NULL DEFAULT 'pending';
-- Values: 'pending' | 'extracting' | 'extracted' | 'verified' | 'pruned'
-- pending: conversation waiting for extraction batch run
-- extracting: currently being processed (locked from other runs)
-- extracted: extraction + MemoryPort.store() succeeded
-- verified: verification confirmed, ready for pruning
-- pruned: context blob compressed or nulled, marked as cold

ALTER TABLE conversations ADD COLUMN extracted_at TIMESTAMP;
-- When extraction completed (NULL until extracted)
-- Used for audit trail and grace period calculations

ALTER TABLE conversations ADD COLUMN memory_id TEXT;
-- UUID of the MemoryPort record (Composite adapter assignment)
-- References result of MemoryPort.store() call; stored for audit trail
-- NULL if extraction incomplete or verification pending

ALTER TABLE conversations ADD COLUMN intent_hash TEXT;
-- SHA256 of the distilled intent snapshot (for dedup & verification)
-- Allows detection of intent drift or re-extraction need
-- NULL if extraction incomplete

-- Indexes for extraction pipeline discovery and efficiency
CREATE INDEX idx_conversations_intent_pending
    ON conversations(workspace_id, created_at)
    WHERE intent_state IN ('pending', 'extracting');
-- Used for "find eligible conversations for extraction batch"

CREATE INDEX idx_conversations_intent_verified
    ON conversations(workspace_id, length(context) DESC)
    WHERE intent_state = 'verified' AND context IS NOT NULL;
-- Used for "find pruning candidates ordered by blob size"

CREATE INDEX idx_conversations_memory_id
    ON conversations(memory_id)
    WHERE memory_id IS NOT NULL;
-- Used for "verify that MemoryPort records are queryable"
