-- Moderation reasons are arbitrary Unicode, including U+0000 in cited names.
-- JSONB cannot represent U+0000, but JSON preserves it as a JSON escape. Keep
-- the same JSON values/API shape without sanitizing or discarding source data.
-- Neither column has a JSONB expression index or operator-dependent consumer.
-- Refinery runs this migration and its history insert in one transaction.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
ALTER TABLE contract_moderation_events
    ALTER COLUMN action TYPE json USING action::json;
ALTER TABLE contract_moderation_identity_state
    ALTER COLUMN state TYPE json USING state::json;
