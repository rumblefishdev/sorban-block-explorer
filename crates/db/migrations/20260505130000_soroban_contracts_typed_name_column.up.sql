-- Task 0156 / ADR 0041 — replace `soroban_contracts.metadata JSONB` with
-- typed `name VARCHAR(256)`. Per ADR 0023 narrowing (codified in ADR 0037),
-- typed columns are preferred over JSONB for closed-shape data. The
-- JSONB column was added forward-looking but only ever needed to carry a
-- single field (`Symbol("name")` extracted from contract storage at
-- deploy time), and it was never written — `extract_contract_deployments`
-- in `crates/xdr-parser/src/state.rs:69` writes `json!({})` for every
-- deployment. The typed column eliminates per-row header overhead
-- (~12 B saved at scale ≈ 120 MB at 10M contracts), simplifies the read
-- path, and aligns the row with the asset registry convention
-- (`assets.name VARCHAR(256)`).
--
-- The dependent generated `search_vector` column (and its GIN index)
-- must be dropped before `metadata`, then recreated reading the typed
-- `name` column instead of `metadata->>'name'`. Postgres does not allow
-- altering a generated column's expression; recreation is the only
-- supported path.
--
-- Data move (`UPDATE … SET name = metadata->>'name'`) is a no-op in
-- current state because every row has `metadata = '{}'` or NULL; the
-- statement is preserved as defence-in-depth so down/up cycles on a
-- populated staging DB never lose data.
--
-- No CONCURRENTLY for the GIN index recreation: this runs against
-- staging/dev DBs pre-traffic (per backfill-execution-plan.md
-- convention used in 20260428000100_add_endpoint_query_indexes.up.sql
-- and 20260430000000_invocations_caller_contract.up.sql).

DROP INDEX idx_contracts_search;

ALTER TABLE soroban_contracts DROP COLUMN search_vector;

ALTER TABLE soroban_contracts ADD COLUMN name VARCHAR(256);

UPDATE soroban_contracts
   SET name = metadata->>'name'
 WHERE metadata IS NOT NULL
   AND metadata ? 'name';

ALTER TABLE soroban_contracts DROP COLUMN metadata;

ALTER TABLE soroban_contracts
    ADD COLUMN search_vector TSVECTOR
    GENERATED ALWAYS AS (
        to_tsvector('simple', COALESCE(name, '') || ' ' || contract_id)
    ) STORED;

CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
