-- Drop secondary (read-path) indexes before backfill.
-- Source of truth for the index list: lore/2-adrs/0037_current-schema-snapshot.md
--
-- NOT dropped here (required by backfill itself):
--   * every PRIMARY KEY
--   * every UNIQUE constraint / partial unique index (used for upserts and FK validation):
--       uq_transactions_hash_created_at, uq_ops_app_identity, uq_lp_snapshots_pool_ledger,
--       nfts (contract_id, token_id), uidx_assets_*, uidx_abc_*,
--       UNIQUE on accounts.account_id, soroban_contracts.contract_id,
--       wasm_interface_metadata.wasm_hash, ledgers.hash
--
-- idx_tx_ledger is included — drop only if the backfill path does NOT look up
-- transactions by ledger_sequence. Comment out the line if it does.

BEGIN;

-- ledgers
DROP INDEX IF EXISTS idx_ledgers_closed_at;

-- accounts
DROP INDEX IF EXISTS idx_accounts_last_seen;
DROP INDEX IF EXISTS idx_accounts_prefix;

-- soroban_contracts
DROP INDEX IF EXISTS idx_contracts_type;
DROP INDEX IF EXISTS idx_contracts_wasm;
DROP INDEX IF EXISTS idx_contracts_search;
DROP INDEX IF EXISTS idx_contracts_prefix;

-- transactions
DROP INDEX IF EXISTS idx_tx_ledger;
DROP INDEX IF EXISTS idx_tx_source_created;
DROP INDEX IF EXISTS idx_tx_has_soroban;

-- operations_appearances
DROP INDEX IF EXISTS idx_ops_app_type;
DROP INDEX IF EXISTS idx_ops_app_source;
DROP INDEX IF EXISTS idx_ops_app_destination;
DROP INDEX IF EXISTS idx_ops_app_contract;
DROP INDEX IF EXISTS idx_ops_app_asset;
DROP INDEX IF EXISTS idx_ops_app_pool;

-- transaction_participants
DROP INDEX IF EXISTS idx_tp_tx;

-- soroban_events_appearances
DROP INDEX IF EXISTS idx_sea_contract_ledger;
DROP INDEX IF EXISTS idx_sea_transaction;

-- soroban_invocations_appearances
DROP INDEX IF EXISTS idx_sia_contract_ledger;
DROP INDEX IF EXISTS idx_sia_transaction;

-- assets
DROP INDEX IF EXISTS idx_assets_type;
DROP INDEX IF EXISTS idx_assets_code_trgm;

-- nfts
DROP INDEX IF EXISTS idx_nfts_collection;
DROP INDEX IF EXISTS idx_nfts_owner;
DROP INDEX IF EXISTS idx_nfts_name_trgm;

-- liquidity_pools
DROP INDEX IF EXISTS idx_pools_asset_a;
DROP INDEX IF EXISTS idx_pools_asset_b;

-- liquidity_pool_snapshots
DROP INDEX IF EXISTS idx_lps_pool;
DROP INDEX IF EXISTS idx_lps_tvl;

-- lp_positions
DROP INDEX IF EXISTS idx_lpp_shares;

-- account_balances_current
DROP INDEX IF EXISTS idx_abc_asset;

-- ---------------------------------------------------------------------------
-- DO NOT drop these — verified 2026-04-24 against crates/indexer/src/handler/
-- persist/write.rs: every one is named (directly or via column tuple) in an
-- `ON CONFLICT` clause. Dropping them makes the corresponding INSERT fail
-- with SQLSTATE 42P10 "no unique or exclusion constraint matching the
-- ON CONFLICT specification":
--
--   * uq_ops_app_identity                  ← write.rs:764
--   * transaction_participants_pkey        ← write.rs:658
--   * soroban_events_appearances_pkey      ← write.rs:860
--   * soroban_invocations_appearances_pkey ← write.rs:978
--   * nft_ownership_pkey                   ← write.rs:1439
-- ---------------------------------------------------------------------------

COMMIT;
