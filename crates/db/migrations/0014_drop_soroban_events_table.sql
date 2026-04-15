-- Drop the soroban_events table entirely.
--
-- This table stored ~3,600 events per ledger (84.6% diagnostic, 15.4% contract)
-- and was 58% of total database size. Event data (topics, data) was already
-- removed in migration 0011 — the remaining thin index (contract_id, event_type,
-- event_index, ledger_sequence, created_at) is redundant because:
--
-- 1. Per-transaction events can be derived from the raw XDR on S3 using
--    ledger_sequence from the transactions table.
-- 2. Per-contract event listing can use soroban_invocations (which has
--    contract_id + transaction_id) as the index, then lazy-fetch from S3.
--
-- Removes the parent partitioned table and all partitions + default.

DROP TABLE IF EXISTS soroban_events CASCADE;
