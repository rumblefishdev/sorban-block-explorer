---
id: '0156'
title: 'Indexer: extract Soroban token name from ContractData (typed `name` column)'
type: FEATURE
status: backlog
related_adr: ['0023', '0027', '0031', '0037']
related_tasks: ['0120', '0124', '0133']
tags: [priority-medium, effort-medium, layer-indexer, layer-db, schema-change]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/ledger_entry_changes.rs
  - crates/indexer/src/handler/persist/write.rs
  - crates/db/migrations/0002_identity_and_ledgers.sql
history:
  - date: '2026-04-23'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0120 future work. 0120 wires Soroban token detection
      but leaves `ExtractedAsset.name` / `.symbol` / `.total_supply` as
      `None`. `upsert_assets_soroban` COALESCE-es NULL so a later run
      can populate without regressing existing rows.
  - date: '2026-05-04'
    status: backlog
    who: stkrolikiewicz
    note: >
      Re-scoped to `name` only. Original draft wanted name+symbol+decimals;
      audit of frontend-overview, endpoint-queries (08/09), and
      docs/architecture showed `symbol` and `decimals` are not consumed
      anywhere in current detail/list views, and `decimals` use case
      (balance scaling) lives entirely under task 0138 which still has
      open design questions on raw vs normalized values. Per minimal-
      schema principle (ADR 0037 narrowing of ADR 0023 Part 3), only
      data that some endpoint actually needs gets persisted at ingest.
      `name` is needed by: (a) `assets.name` column already in schema
      (asset list + detail labels per endpoint-queries 08/09), and
      (b) `soroban_contracts.search_vector` GENERATED column which
      reads `metadata->>'name'` (migration 0002:58-60) — without this
      task, FTS search on Soroban contract names returns nothing
      because metadata is `{}`. If 0138 later concludes it needs
      `decimals` for balance scaling, spawn a follow-up; do not
      pre-store unused fields.
  - date: '2026-05-04'
    status: backlog
    who: stkrolikiewicz
    note: >
      Scope expanded to include schema migration: replace
      `soroban_contracts.metadata JSONB` with typed `name VARCHAR(256)`
      column. Reasons: (1) per ADR 0023 narrowing, JSONB is reserved for
      genuinely open metadata shapes; `name` is closed-shape (single
      predictable string), VARCHAR is the documented preference;
      (2) ~12 B/row storage saved (~120 MB at 10M contracts) by
      eliminating JSONB header + key overhead for a single-field
      payload; (3) read path simpler — `sc.name` vs `sc.metadata->>'name'`
      in 22_get_search.sql + GENERATED `search_vector`; (4) atomic
      change — 0156 already touches the write path
      (`upsert_contracts_returning_id`) and the search query, so the
      schema migration costs nothing extra in PR review surface.
      Migration is a no-op data move (current rows have `metadata = {}`
      or NULL; nothing to copy). Effort small → medium. ADR is to be
      drafted by author at pickup (amendment to ADR 0023 or new ADR
      formalizing the typed-name decision).
---

# Indexer: extract Soroban token name from ContractData at deploy time

## Summary

After 0120, Soroban-native (WASM-based) `Fungible` contracts produce an `assets` row at deploy time with `asset_type = Soroban` and `contract_id` set, but `name` is NULL — `extract_contract_deployments` writes `metadata = json!({})` (state.rs:69). This task populates `name` at ingest time by reading the contract's persistent storage entries emitted in the same ledger as the deployment, writing it both to `assets.name` (display label) and to `soroban_contracts.name` (consumed by the GENERATED `search_vector`).

Bundled schema change: replace `soroban_contracts.metadata JSONB` with typed `soroban_contracts.name VARCHAR(256)`. JSONB carried only `name` (single closed-shape field) since deploy and is therefore an over-general persistence layer per ADR 0023 narrowing — typed column saves ~12 B/row, simplifies the read path, and aligns the row with the asset/account row philosophy. Migration is a no-op data move (existing rows are `{}` or NULL).

## Context

Parent task 0120 (merged) covers the **detection + classification** side of Soroban token handling. It deliberately defers metadata population to avoid scope creep while the classification + persist wiring lands.

Sibling task 0124 (active) addresses a different enrichment path: a scheduled Lambda that scans `assets WHERE metadata IS NULL` and fetches SEP-1 TOML from issuer home_domains. That Lambda is the right fit for **classic / SAC** tokens whose metadata is off-chain.

For **Soroban-native** tokens the name is **on-chain**: the standard OpenZeppelin / SDK pattern stores `name` as a persistent ContractData entry keyed by `Symbol("name")`. This entry shows up in the ledger as an `ExtractedLedgerEntryChange` record with `entry_type = "contract_data"` during the contract's init transaction. This task reads it inline.

### Why not symbol / decimals?

Audit (2026-05-04) of consumers showed:

- **`symbol`** — not in `assets` schema (no column), not projected by `08_get_assets_list.sql` or `09_get_assets_by_id.sql`, not mentioned in frontend-overview token detail/list sections (lines 437, 448-449), not used by search. No consumer → not extracted.
- **`decimals`** — not in `assets` schema, not in current endpoint-queries, not in `docs/architecture/**`. Single potential consumer is task 0138 (contract balance extraction), which has unresolved design question on raw-i128 vs normalized values (0138 lines 56-58, 70-72). If 0138 picks the normalized path, spawn a `decimals` follow-up; otherwise no consumer exists.

### Why two write targets for `name`

`soroban_contracts.search_vector` is `TSVECTOR GENERATED ALWAYS AS (to_tsvector('simple', COALESCE(<name source>, '') || ' ' || contract_id)) STORED` (migration 0002:58-60). After this task's migration the source becomes `name` (typed column) instead of `metadata->>'name'`. FTS search on contract names **requires** `name` to be populated. Independently, `assets.name` is the column the asset detail/list endpoints read (per endpoint-queries 08/09). Indexer writes both in the same transaction so the targets stay consistent.

### Why typed VARCHAR instead of keeping JSONB

Per ADR 0023 (narrowed by ADR 0037): "typed columns preferred over JSONB for closed domains; JSONB reserved for genuinely open metadata shapes." `soroban_contracts.metadata` was originally JSONB anticipating multi-field metadata, but in practice only `name` is needed (this task's audit confirmed neither schema nor any consumer reads `symbol` / `decimals` / other fields). A single-field JSONB pays:

- Header + key overhead per row (`{"name":"X"}` ≈ 22 B vs VARCHAR `"X"` ≈ 10 B) — at 10M contracts ≈ 120 MB saved.
- Per-read parse cost on every `metadata->>'name'` lookup (search query + GENERATED column recompute).
- Type-safety loss: arbitrary JSONB shape vs `VARCHAR(256)` constraint.

If a future task (e.g. 0138 if it concludes it needs `decimals`) adds another field, `ADD COLUMN <field> <type>` is the path; do not re-introduce a catch-all JSONB. The type system then forces an explicit review of every new column rather than letting fields silently accrete.

## Implementation

### 1. Schema migration (`crates/db/migrations/NNNN_*.sql`)

```sql
-- Drop the GENERATED column first (it depends on metadata).
ALTER TABLE soroban_contracts DROP COLUMN search_vector;
-- Replace metadata JSONB with typed name VARCHAR(256).
ALTER TABLE soroban_contracts ADD COLUMN name VARCHAR(256);
-- No-op data move: aktualnie wszystkie rows mają metadata = {} lub NULL.
-- Pozostawione jako defence-in-depth na wypadek istniejących wpisów.
UPDATE soroban_contracts SET name = metadata->>'name'
  WHERE metadata IS NOT NULL AND metadata ? 'name';
ALTER TABLE soroban_contracts DROP COLUMN metadata;
-- Re-create GENERATED search_vector reading the typed column.
ALTER TABLE soroban_contracts ADD COLUMN search_vector TSVECTOR
  GENERATED ALWAYS AS (
    to_tsvector('simple', COALESCE(name, '') || ' ' || contract_id)
  ) STORED;
-- Re-create GIN index.
CREATE INDEX idx_contracts_search ON soroban_contracts USING GIN (search_vector);
```

### 2. `ExtractedContractDeployment` shape change

`crates/xdr-parser/src/state.rs:69` currently writes `metadata: json!({})`. Replace with:

```rust
pub struct ExtractedContractDeployment {
    // ... existing fields ...
    pub name: Option<String>,   // was: pub metadata: serde_json::Value
}
```

Extend `extract_contract_deployments` to scan adjacent contract_data changes for the standard `Symbol("name")` storage key and populate `name` directly. Decoding follows the OpenZeppelin Stellar contracts library's `FungibleToken` reference implementation (see classification code's references).

### 3. Thread `name` into `ExtractedAsset`

Update `detect_assets` (task 0120) so the Fungible branch reads `deployment.name` and places it on the emitted `ExtractedAsset.name`. `upsert_assets_soroban` already handles partial data via `COALESCE(EXCLUDED.name, assets.name)`.

### 4. `soroban_contracts.name` write

`upsert_contracts_returning_id` (`crates/indexer/src/handler/persist/write.rs:393-426`) currently passes `metadatas: Vec<Option<Value>>` and binds `$8::JSONB[]`. Change to `names: Vec<Option<String>>` bound as `$8::VARCHAR[]`; update the `INSERT ... ON CONFLICT DO UPDATE SET metadata = COALESCE(EXCLUDED.metadata, soroban_contracts.metadata)` clause to `name = COALESCE(EXCLUDED.name, soroban_contracts.name)`. The GENERATED `search_vector` recomputes on the new value.

`ContractRow` struct (in same file or staging.rs:503-501 region) — drop `metadata: Option<Value>`, add `name: Option<String>`.

### 5. Search query + API repository update

- `docs/architecture/database-schema/endpoint-queries/22_get_search.sql:82` — replace `COALESCE(sc.metadata->>'name', '')` with `COALESCE(sc.name, '')`.
- `crates/api/src/search/queries.rs` (added by 0053 PR #155) — re-port the SQL change verbatim.
- Any other repository that projects `sc.metadata->>'name'` — sweep with `grep` and update.

### 6. Bridge path — late-WASM contracts

Late-WASM contracts (the bridge path in 0120) were deployed in an earlier ledger; their ContractData changes are not in-memory during the reclassification. Two options:

**Option A (preferred):** on the next ledger that touches the contract (any invocation), scan the changes for `contract_data` keys on that contract id and backfill via `UPDATE assets SET name = COALESCE(name, …)` plus the matching `UPDATE soroban_contracts SET name = COALESCE(name, …)`.

**Option B:** extend 0124's scheduled enrichment Lambda to decode on-chain storage for Soroban tokens in addition to SEP-1 TOML. Keeps the indexer hot path lean.

Decision deferred to implementation; documenting both for the author.

### 7. ADR

Author drafts an ADR at pickup, either:

- **Amendment to ADR 0023** documenting the narrowing principle in practice (closed-shape fields use typed columns; soroban_contracts is the worked example), OR
- **New ADR (0040 or next free)** "soroban_contracts: typed `name` column replaces JSONB metadata," referencing ADR 0023 + 0037 as basis.

### 8. Tests

- **Unit** (`state.rs`): deployment with ContractData changes containing `Symbol("name")` = "MyToken" produces `deployment.name = Some("MyToken")`.
- **Unit** (`state.rs`): `detect_assets` propagates name from deployment into `ExtractedAsset.name`.
- **Integration**: synthetic ledger with Fungible deploy + ContractData changes → `assets.name` populated AND `soroban_contracts.name` populated AND `search_vector` matches a query for the name.
- **Migration test**: verify migration up/down doesn't lose data on a row with a populated metadata.name (defence in depth even though current rows are empty).

## Acceptance Criteria

- [ ] Migration drops `soroban_contracts.metadata` JSONB and dependent `search_vector`, adds `name VARCHAR(256)`, recreates `search_vector GENERATED ALWAYS AS (to_tsvector('simple', COALESCE(name, '') || ' ' || contract_id)) STORED` plus `idx_contracts_search` GIN index.
- [ ] Migration includes the no-op `UPDATE … SET name = metadata->>'name' WHERE metadata ? 'name'` as defence in depth before drop.
- [ ] `ExtractedContractDeployment` exposes `name: Option<String>` instead of `metadata: serde_json::Value`.
- [ ] `extract_contract_deployments` populates `deployment.name` when the standard `Symbol("name")` ContractData key is present.
- [ ] `detect_assets` threads the value into `ExtractedAsset.name`.
- [ ] `upsert_assets_soroban` writes non-NULL `assets.name` for Fungible deploys with on-chain name.
- [ ] `upsert_contracts_returning_id` writes `name` to `soroban_contracts.name` (UNNEST `$8::VARCHAR[]` instead of `$8::JSONB[]`); GENERATED `search_vector` recomputes (verified via integration test that an FTS query for the name matches).
- [ ] `22_get_search.sql:82` and `crates/api/src/search/queries.rs` updated: `metadata->>'name'` → `name`. Sweep via `grep "metadata->>" crates/` to catch other repositories.
- [ ] `docs/architecture/database-schema/database-schema-overview.md` updated per ADR 0032: soroban_contracts row reflects the new typed `name` column; references to `metadata->>'name'` removed.
- [ ] ADR drafted: amendment to ADR 0023 OR new ADR formalizing the typed-name decision.
- [ ] Late-WASM path — either Option A implemented, or 0124 scope explicitly extended (not both).
- [ ] No new schema columns beyond `soroban_contracts.name`; `assets.symbol` / `assets.decimals` are NOT introduced.
- [ ] Unit + integration coverage as above.
- [ ] Does NOT regress SAC / classic token paths (name typically already populated from asset_code / SEP-1 metadata).

## Future Work

- If post-launch UX feedback ever surfaces a need for token `symbol` (separate from `asset_code`), spawn a follow-up adding `symbol VARCHAR(N)`. No current consumer.

`decimals` follow-up was previously listed here pending task 0138's design decision. Task 0138 was archived 2026-05-05 as scope-out per current technical design (Soroban contract token balances are explicitly excluded from `account_balances_current`). With no consumer for `decimals` value, the follow-up is removed.
