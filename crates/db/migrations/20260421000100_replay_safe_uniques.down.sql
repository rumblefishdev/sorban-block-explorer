ALTER TABLE operations                DROP CONSTRAINT IF EXISTS uq_operations_tx_order;
ALTER TABLE liquidity_pool_snapshots  DROP CONSTRAINT IF EXISTS uq_lp_snapshots_pool_ledger;
