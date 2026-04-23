-- Natural-key UNIQUE constraints for replay-safe inserts (task 0149).
--
-- Partitioned UNIQUE constraints must include all partition-key columns;
-- `created_at` is the partition key on all four tables. The constraints turn
-- previously append-only inserts into idempotent ON CONFLICT DO NOTHING
-- upserts — a replay of the same ledger produces zero duplicate rows.

ALTER TABLE operations
    ADD CONSTRAINT uq_operations_tx_order
    UNIQUE (transaction_id, application_order, created_at);

-- soroban_events_appearances (ADR 0033) and soroban_invocations_appearances
-- (ADR 0034) get replay idempotency for free: their primary keys
-- (contract_id, transaction_id, ledger_sequence, created_at) are already the
-- natural keys of an appearance row, so no extra constraint is needed here.

ALTER TABLE liquidity_pool_snapshots
    ADD CONSTRAINT uq_lp_snapshots_pool_ledger
    UNIQUE (pool_id, ledger_sequence, created_at);
