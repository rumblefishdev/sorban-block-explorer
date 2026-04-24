-- Natural-key UNIQUE constraints for replay-safe inserts (task 0149).
--
-- Partitioned UNIQUE constraints must include all partition-key columns;
-- `created_at` is the partition key on all four tables. The constraints turn
-- previously append-only inserts into idempotent ON CONFLICT DO NOTHING
-- upserts — a replay of the same ledger produces zero duplicate rows.

-- operations_appearances (task 0163) ships its natural-key UNIQUE
-- (uq_ops_app_identity, NULLS NOT DISTINCT) inline in migration 0003;
-- no extra constraint is needed here.
--
-- soroban_events_appearances (ADR 0033) and soroban_invocations_appearances
-- (ADR 0034) get replay idempotency for free: their primary keys
-- (contract_id, transaction_id, ledger_sequence, created_at) are already the
-- natural keys of an appearance row, so no extra constraint is needed here.

ALTER TABLE liquidity_pool_snapshots
    ADD CONSTRAINT uq_lp_snapshots_pool_ledger
    UNIQUE (pool_id, ledger_sequence, created_at);
