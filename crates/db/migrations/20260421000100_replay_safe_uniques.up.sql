-- Natural-key UNIQUE constraints for replay-safe inserts (task 0149).
--
-- Partitioned UNIQUE constraints must include all partition-key columns;
-- `created_at` is the partition key on all four tables. The constraints turn
-- previously append-only inserts into idempotent ON CONFLICT DO NOTHING
-- upserts — a replay of the same ledger produces zero duplicate rows.

ALTER TABLE operations
    ADD CONSTRAINT uq_operations_tx_order
    UNIQUE (transaction_id, application_order, created_at);

ALTER TABLE soroban_events
    ADD CONSTRAINT uq_soroban_events_tx_index
    UNIQUE (transaction_id, event_index, created_at);

ALTER TABLE soroban_invocations
    ADD CONSTRAINT uq_soroban_invocations_tx_index
    UNIQUE (transaction_id, invocation_index, created_at);

ALTER TABLE liquidity_pool_snapshots
    ADD CONSTRAINT uq_lp_snapshots_pool_ledger
    UNIQUE (pool_id, ledger_sequence, created_at);
