-- A DPNS alias includes both its label and parent domain name, so valid
-- fully qualified aliases can exceed the old 64-character projection limit.
-- Widen storage without truncation, normalization, or discarded transactions.
-- Refinery runs this migration and its history insert in one transaction.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
ALTER TABLE identity_aliases ALTER COLUMN alias TYPE text;
-- Runtime rollback must retain this widened column and migration history:
-- narrowing after long aliases arrive would fail or require data loss.
