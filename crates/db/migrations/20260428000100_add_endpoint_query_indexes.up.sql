-- Five indexes surfaced by per-endpoint EXPLAIN audits (task 0167 +
-- PR 136 review on E02 Statement B variant 2). All flagged inline as
-- `INDEX GAP:` comments inside
-- `docs/architecture/database-schema/endpoint-queries/`.
--
-- These run as plain `CREATE INDEX` (no `CONCURRENTLY`). Three of the
-- five target partitioned parents (`transactions`,
-- `soroban_invocations_appearances`, `soroban_events_appearances`),
-- and Postgres forbids `CONCURRENTLY` on partitioned tables — index
-- creation cascades to children, each holding an AccessExclusiveLock
-- for its own build. The migration is intended to run post-restore on
-- a freshly populated staging DB before Galexie or any live traffic is
-- pointed at it (per `lore/3-wiki/backfill-execution-plan.md`), so the
-- brief lock window is moot. Mixing CONCURRENTLY for the two
-- non-partitioned tables would have made this file inconsistent
-- without buying anything in the deployment scenario it actually
-- targets.

-- E2 GET /transactions — no-filter keyset on (created_at DESC, id DESC).
-- Without it the planner falls back to per-partition seq + sort.
-- See 02_get_transactions_list.sql header (INDEX GAP — Statement A).
CREATE INDEX IF NOT EXISTS idx_tx_keyset
    ON transactions (created_at DESC, id DESC);

-- E15 GET /nfts — collection_name filter is currently exact `=` against
-- a btree (`idx_nfts_collection`); the endpoint contract wants ILIKE.
-- The trigram GIN unblocks ILIKE; the existing btree stays as the
-- equality path, complementary not redundant.
-- See 15_get_nfts_list.sql:30.
CREATE INDEX IF NOT EXISTS idx_nfts_collection_trgm
    ON nfts USING GIN (collection_name gin_trgm_ops);

-- E18 GET /liquidity-pools — keyset on (created_at_ledger DESC, pool_id DESC).
-- Pre-emptive: liquidity_pools is small enough today that a heap scan +
-- sort is tolerable, but cost grows linearly with pool count.
-- See 18_get_liquidity_pools_list.sql:31.
CREATE INDEX IF NOT EXISTS idx_pools_created_at_ledger
    ON liquidity_pools (created_at_ledger DESC, pool_id DESC);

-- E2 Statement B (variant 2) — broad-match contract filter UNIONs three
-- appearance tables and keyset-orders by (created_at DESC, transaction_id DESC).
-- The existing idx_sia_contract_ledger / idx_sea_contract_ledger lead
-- with ledger_sequence and don't carry the keyset shape — on a popular
-- contract with millions of rows that forces a sort step.
-- See 02_get_transactions_list.sql header (INDEX GAP — Statement B).
CREATE INDEX IF NOT EXISTS idx_sia_contract_keyset
    ON soroban_invocations_appearances
       (contract_id, created_at DESC, transaction_id DESC);

CREATE INDEX IF NOT EXISTS idx_sea_contract_keyset
    ON soroban_events_appearances
       (contract_id, created_at DESC, transaction_id DESC);
