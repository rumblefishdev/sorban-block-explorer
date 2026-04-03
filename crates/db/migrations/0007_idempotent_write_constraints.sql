-- Add unique constraints for idempotent writes (task 0028).
-- Enables INSERT ON CONFLICT DO NOTHING for all immutable tables.

-- Operations: unique per transaction + order.
-- Partition key (transaction_id) is already in the constraint.
ALTER TABLE operations
    ADD CONSTRAINT uq_operations_tx_order UNIQUE (transaction_id, application_order);

-- Events: add event_index column for dedup within a transaction.
-- Partition key (created_at) must be in the unique constraint.
ALTER TABLE soroban_events
    ADD COLUMN event_index SMALLINT NOT NULL DEFAULT 0;

ALTER TABLE soroban_events
    ADD CONSTRAINT uq_events_tx_index UNIQUE (transaction_id, event_index, created_at);

-- Invocations: add invocation_index column for dedup within a transaction.
-- Partition key (created_at) must be in the unique constraint.
ALTER TABLE soroban_invocations
    ADD COLUMN invocation_index SMALLINT NOT NULL DEFAULT 0;

ALTER TABLE soroban_invocations
    ADD CONSTRAINT uq_invocations_tx_index UNIQUE (transaction_id, invocation_index, created_at);
