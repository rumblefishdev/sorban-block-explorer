-- Recreate secondary indexes after backfill.
-- DDL mirrors lore/2-adrs/0037_current-schema-snapshot.md 1:1.
--
-- Non-CONCURRENTLY: assumes the DB is idle (test/backfill window).
-- For a live DB, replace each `CREATE INDEX` with `CREATE INDEX CONCURRENTLY`
-- and remove the surrounding BEGIN/COMMIT (CONCURRENTLY cannot run in a tx).
--
-- Tip: `SET maintenance_work_mem = '2GB';` before running speeds large builds.

BEGIN;

-- ledgers
CREATE INDEX idx_ledgers_closed_at ON ledgers (closed_at DESC);

-- accounts
CREATE INDEX idx_accounts_last_seen ON accounts (last_seen_ledger DESC);
CREATE INDEX idx_accounts_prefix    ON accounts (account_id text_pattern_ops);

-- soroban_contracts
CREATE INDEX idx_contracts_type   ON soroban_contracts (contract_type);
CREATE INDEX idx_contracts_wasm   ON soroban_contracts (wasm_hash) WHERE wasm_hash IS NOT NULL;
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
CREATE INDEX idx_contracts_prefix ON soroban_contracts (contract_id text_pattern_ops);

-- transactions (partitioned — creates a matching index on each partition)
CREATE INDEX idx_tx_ledger         ON transactions (ledger_sequence);
CREATE INDEX idx_tx_source_created ON transactions (source_id, created_at DESC);
CREATE INDEX idx_tx_has_soroban    ON transactions (created_at DESC) WHERE has_soroban;

-- operations_appearances (partitioned)
CREATE INDEX idx_ops_app_type        ON operations_appearances (type, created_at DESC);
CREATE INDEX idx_ops_app_source      ON operations_appearances (source_id, created_at DESC)      WHERE source_id IS NOT NULL;
CREATE INDEX idx_ops_app_destination ON operations_appearances (destination_id, created_at DESC) WHERE destination_id IS NOT NULL;
CREATE INDEX idx_ops_app_contract    ON operations_appearances (contract_id, created_at DESC)    WHERE contract_id IS NOT NULL;
CREATE INDEX idx_ops_app_asset       ON operations_appearances (asset_code, asset_issuer_id, created_at DESC) WHERE asset_code IS NOT NULL;
CREATE INDEX idx_ops_app_pool        ON operations_appearances (pool_id, created_at DESC)        WHERE pool_id IS NOT NULL;

-- transaction_participants (partitioned)
CREATE INDEX idx_tp_tx ON transaction_participants (transaction_id);

-- soroban_events_appearances (partitioned)
CREATE INDEX idx_sea_contract_ledger ON soroban_events_appearances (contract_id, ledger_sequence DESC, created_at DESC);
CREATE INDEX idx_sea_transaction     ON soroban_events_appearances (transaction_id, created_at DESC);

-- soroban_invocations_appearances (partitioned)
CREATE INDEX idx_sia_contract_ledger ON soroban_invocations_appearances (contract_id, ledger_sequence DESC);
CREATE INDEX idx_sia_transaction     ON soroban_invocations_appearances (transaction_id);

-- assets
CREATE INDEX idx_assets_type      ON assets (asset_type);
CREATE INDEX idx_assets_code_trgm ON assets USING GIN (asset_code gin_trgm_ops);

-- nfts
CREATE INDEX idx_nfts_collection ON nfts (collection_name);
CREATE INDEX idx_nfts_owner      ON nfts (current_owner_id);
CREATE INDEX idx_nfts_name_trgm  ON nfts USING GIN (name gin_trgm_ops);

-- liquidity_pools
CREATE INDEX idx_pools_asset_a ON liquidity_pools (asset_a_code, asset_a_issuer_id);
CREATE INDEX idx_pools_asset_b ON liquidity_pools (asset_b_code, asset_b_issuer_id);

-- liquidity_pool_snapshots (partitioned)
CREATE INDEX idx_lps_pool ON liquidity_pool_snapshots (pool_id, created_at DESC);
CREATE INDEX idx_lps_tvl  ON liquidity_pool_snapshots (tvl DESC) WHERE tvl IS NOT NULL;

-- lp_positions
CREATE INDEX idx_lpp_shares ON lp_positions (pool_id, shares DESC) WHERE shares > 0;

-- account_balances_current
CREATE INDEX idx_abc_asset ON account_balances_current (asset_code, issuer_id) WHERE asset_code IS NOT NULL;

COMMIT;

-- Refresh planner statistics after rebuild.
ANALYZE;
