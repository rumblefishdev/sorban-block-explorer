ALTER TABLE operations                DROP CONSTRAINT IF EXISTS uq_operations_tx_order;
ALTER TABLE soroban_events            DROP CONSTRAINT IF EXISTS uq_soroban_events_tx_index;
ALTER TABLE soroban_invocations       DROP CONSTRAINT IF EXISTS uq_soroban_invocations_tx_index;
ALTER TABLE liquidity_pool_snapshots  DROP CONSTRAINT IF EXISTS uq_lp_snapshots_pool_ledger;
