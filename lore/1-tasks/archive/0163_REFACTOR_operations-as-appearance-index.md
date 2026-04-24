---
id: '0163'
title: 'Refactor: operations → operations_appearances (appearance index)'
type: REFACTOR
status: completed
related_adr: ['0027', '0033', '0034']
related_tasks: []
tags: [layer-db, layer-indexer, operations, appearance-index, schema-migration]
links: []
history:
  - date: 2026-04-24
    status: active
    who: fmazur
    note: 'Task created and activated'
  - date: 2026-04-24
    status: completed
    who: fmazur
    note: >
      Refactor landed. Rewrite-in-place migration (0003 + 0006 + replay-safe
      uniques); indexer staging moved to in-memory `HashMap<identity, count>`
      aggregation; domain `Operation` → `OperationAppearance`; 6 integration
      tests passing; fresh backfill yields 55 506 rows for 76 882 operations
      (28% compression), type-14 collapses to 1.000 row per transaction
      carrying CCB ops (179/179). DB size 70 → 67 MB after dropping redundant
      `idx_ops_app_tx`. No API breakage — per-op detail already served from
      XDR via `stellar_archive` path.
---

# Refactor: `operations` → `operations_appearances`

## Summary

Slim the `operations` table down to an appearance index (same pattern as
`soroban_events_appearances` / `soroban_invocations_appearances`, ADR 0033/0034)
and rename it to `operations_appearances`. Per-operation detail (amounts, order,
memo, claimants, function args, predicates, …) is re-materialised in the API
from XDR archived in S3 — the DB only records _that_ an operation of a given
shape occurred in a given transaction.

## Status: Completed

**Final state:** Rewrite-in-place migration landed. Indexer staging moved to
in-memory `HashMap<identity, count>` aggregation. Domain model renamed
`Operation` → `OperationAppearance`. All 6 persist integration tests green.
Fresh backfill: 55 506 rows for 76 882 operations (28% compression); type-14
CCB collapses to 1.000 row per CCB-carrying transaction. No API breakage —
per-op detail already served from XDR via the `stellar_archive` path.

## Context

Current `operations` table (`crates/db/migrations/0003_transactions_and_operations.sql`,
ADR 0027 Part I §5) carries typed `transfer_amount NUMERIC(28,7)` and
`application_order SMALLINT` columns that **no API endpoint reads** (confirmed
by repo-wide scan — `grep -rn "FROM operations" crates/api` returns zero).
The only API code-path that surfaces per-op detail is
`crates/api/src/stellar_archive/extractors.rs:274`, which already sources
`application_order` and everything else from XDR via
`xdr_parser::extract_operations` — not from the DB.

`crates/domain/src/operation.rs` is dead (no consumer beyond `OperationType`
enum re-export).

Conclusion: the table is _already_ an appearance index in practice; it just
still carries two write-only columns and a wide PK that prevents collapsing
duplicates. Type 14 (`CREATE_CLAIMABLE_BALANCE`) is the worst case in the
current dataset — 102 607 rows, all-NULL on typed columns, would collapse to
~1 200 rows (one per transaction).

## Implementation Plan

### Step 1 — Migration

- Add `crates/db/migrations/<ts>_operations_appearances.up.sql`:
  - `CREATE TABLE operations_appearances` with columns: `id BIGSERIAL`,
    `transaction_id`, `type SMALLINT`, `source_id`, `destination_id`,
    `contract_id`, `asset_code`, `asset_issuer_id`, `pool_id`, `ledger_sequence`,
    `amount BIGINT NOT NULL`, `created_at TIMESTAMPTZ NOT NULL`.
  - PK `(id, created_at)`, partition `RANGE (created_at)`, FKs matching current
    `operations`.
  - Indexes mirroring current `idx_ops_*` (asset, contract, destination, pool,
    tx, type) — re-evaluate which are still needed after the shape change.
  - `ck_ops_type_range`, `ck_ops_pool_id_len` carried over.
- `.down.sql` drops the new table (no data preservation — current DB is local).

### Step 2 — Indexer rewrite

- `crates/indexer/src/handler/persist/staging.rs`:
  - Drop `application_order` and `transfer_amount` from `OpRow`.
  - Drop `OpTyped::transfer_amount` / `pool_id_hex`-with-amount paths;
    `OpTyped` keeps only the identity columns.
  - Aggregate ops in-memory with `HashMap<key, i64>` keyed by
    `(tx_hash, type, source, destination, contract, asset_code, asset_issuer,
pool_id, ledger_sequence, created_at)`, incrementing `amount`.
- `crates/indexer/src/handler/persist/write.rs`:
  - Rename table in INSERT. Bind `amount` vec. Drop `application_order` and
    `transfer_amount` binds.
  - Reuse the appearance-index `ON CONFLICT DO NOTHING` replay pattern if
    natural-key PK is adopted (open question — see Notes).

### Step 3 — Domain + tests

- Update `crates/domain/src/operation.rs` to the new shape (or delete if still
  unused after rewrite).
- Update `crates/indexer/tests/persist_integration.rs` — SQL references and
  assertions currently checking `application_order` / `transfer_amount`.

### Step 4 — Docs

- Update `docs/architecture/technical-design-general-overview.md:880-891`
  (still shows pre-ADR-0027 `details JSONB` schema anyway — scope overlap
  with 0155).
- Update `docs/architecture/database-schema/**` if it references the old
  shape.

### Step 5 — Verify no API breakage

- Re-run `grep -rn "operations" crates/api` after rename to confirm nothing
  compiles against the old name. Expected: zero hits outside `stellar_archive`
  (XDR-sourced).

## Acceptance Criteria

- [x] New `operations_appearances` table with the agreed columns + partitions
- [x] Indexer writes collapsed rows with `amount = COUNT(*)` per identity key
- [x] Type 14 rowcount drops to ~one per transaction carrying a CCB op —
      measured 1.000 (179 rows / 179 distinct transactions)
- [x] Integration test `persist_integration.rs` passes against new schema —
      6/6 tests green
- [x] `cargo check` clean across workspace
- [x] No references to `transfer_amount` / `application_order` remain
      outside `stellar_archive` (XDR path) and xdr-parser — verified;
      leftover matches are all `transactions.application_order` (a
      different column representing tx position in ledger)
- [x] Overview doc updated to new schema —
      `docs/architecture/database-schema/database-schema-overview.md` §4.3
      plus 6 textual refs, and `technical-design-general-overview.md` §6.3
      plus §6.12 partitioning list

## Implementation Notes

Rewrite-in-place migration strategy (no production DB yet, matches ADR 0030/0031
pattern already used for `0004_soroban_activity.sql`). Files touched:

- `crates/db/migrations/0003_transactions_and_operations.sql` — new
  `operations_appearances` DDL replacing `operations`
- `crates/db/migrations/0006_liquidity_pools.sql` — deferred FK rename
  `fk_ops_pool_id` → `fk_ops_app_pool_id`
- `crates/db/migrations/20260421000100_replay_safe_uniques.{up,down}.sql` —
  `uq_operations_tx_order` dropped (natural-key UNIQUE now lives inline in 0003)
- `crates/db-partition-mgmt/src/lib.rs` — `TIME_PARTITIONED_TABLES` + regression
  test updated
- `crates/backfill-bench/src/main.rs` — local partition bootstrap updated
- `crates/domain/src/operation.rs` — `Operation` → `OperationAppearance`
  (slim shape, amount column added)
- `crates/xdr-parser/src/types.rs` — `ExtractedOperation` doc comments
  updated to reflect new persistence target and the fact that
  `operation_index` is no longer persisted
- `crates/indexer/src/handler/persist/staging.rs` — `OpRow` slimmed,
  `OpTyped::from_details` reduced to identity columns only,
  `stroops_as_numeric` helper removed (unused after refactor), new in-memory
  `HashMap<OpIdentity, i64>` aggregation collapses identical-shape ops per
  transaction
- `crates/indexer/src/handler/persist/write.rs` — INSERT rewritten for the
  slim schema; `ON CONFLICT ON CONSTRAINT uq_ops_app_identity DO NOTHING`
- `crates/indexer/tests/persist_integration.rs` — table name + CTEs
  updated; new assert on `amount == 1` for fixture ops; ORDER BY switched
  from `application_order` to `type`
- `docs/architecture/database-schema/database-schema-overview.md` and
  `docs/architecture/technical-design-general-overview.md` — schema
  blocks and textual references updated

## Design Decisions

### From Plan

1. **Appearance-index pattern.** Operations table collapses to
   `soroban_events_appearances` / `soroban_invocations_appearances` shape:
   one row per distinct identity per transaction, `amount BIGINT` counts
   collapsed duplicates. Per-op detail (transfer amount, application order,
   memo, claimants, function args, predicates) is re-materialised by the
   API from XDR archived in S3 — never stored in the DB.

2. **Rewrite-in-place over forward migration.** No production DB yet
   (owner-confirmed); `0004_soroban_activity.sql` header already blesses
   this pattern. Cleaner than a new timestamped migration that would carry
   a `DROP TABLE operations CASCADE` alongside the rewrite.

### Emerged

3. **PK = `BIGSERIAL id` + `UNIQUE NULLS NOT DISTINCT` natural key.**
   Task planning listed three options. Picked (a): `PRIMARY KEY (id, created_at)`
   for partitioning compatibility, plus wide `uq_ops_app_identity UNIQUE
NULLS NOT DISTINCT (transaction_id, type, source_id, …)`. PG 16 in local
   env supports NULLS NOT DISTINCT (introduced in 15). This makes NULL-heavy
   shapes (type-14 CCB with source inherited from tx) idempotent under
   `ON CONFLICT DO NOTHING` on replay. Trade-off accepted: the wide UNIQUE
   is ~5.5 MB on current data and will dominate index mass at scale, but
   it's the correct semantics.

4. **Dropped `idx_ops_app_tx (transaction_id)` after the fact.** Originally
   in the plan; removed after EXPLAIN confirmed that
   `uq_ops_app_identity` is a valid prefix index for `WHERE transaction_id = X`
   (leftmost column). Benchmarked on 55 506 rows: plan cost +23%, runtime
   difference negligible (6 vs 7 buffers, 0.130 vs 0.098 ms). Saved 1 400 kB
   now, scaling to ~100 MB at mainnet scale. Decision is reversible via
   `CREATE INDEX CONCURRENTLY` on live partitions. Left a comment in
   `0003_*.sql` documenting the rationale.

5. **Indexer aggregates at staging, not at write.** Implemented
   `HashMap<OpIdentity, i64>` in `staging.rs` flatten loop, emitting one
   `OpRow` per identity with final `amount`. Write layer stays a dumb bulk
   INSERT. Alternative was SQL-side `GROUP BY` on a staging table — more
   round-trips, same result.

6. **Natural-key UNIQUE moved out of `20260421000100_replay_safe_uniques`.**
   That migration added `uq_operations_tx_order` for the old schema (task
   0149). Since the new natural-key UNIQUE lives inline in `0003`, the
   replay-safe-uniques migration no longer touches the operations table.
   Cleaner than leaving a no-op ALTER behind.

7. **`ExtractedOperation.details` comment clarified.** Doc comment in
   `xdr-parser/src/types.rs` used to say "stored as JSONB in PostgreSQL" —
   now clarifies that `details` is consumed by staging to extract identity
   columns, not persisted as JSONB anywhere (was already true pre-0163
   after ADR 0027, but stale doc comment could confuse readers).

## Issues Encountered

- **Duplicate `#[derive(Default)]` on `OpTyped`.** First edit replaced the
  doc comment block but kept the original `#[derive(Default)]` line while
  also adding a new one. `cargo check` caught it immediately
  (`E0119: conflicting implementations of trait Default`). Fixed by
  collapsing to a single derive.

- **`\gset` not available via `docker exec psql -c`.** Tried to capture a
  `transaction_id` via `\gset` for an EXPLAIN test; `-c` mode doesn't
  support backslash-meta. Fell back to shell-side capture (`psql -tA -c`
  into a bash variable). Only relevant for one-off diagnostic, not a
  regression.

## Future Work

- **Index audit at scale.** Current 6 indexes on `operations_appearances`
  are all 0-scan in dev because the API doesn't query this table yet.
  Revisit once mainnet backfill is running and API endpoints are wired;
  some (e.g. `idx_ops_app_type`) may be droppable if never hit.
- **`domain::OperationAppearance` still unused.** Kept as the canonical
  read-path shape for when the API starts reading from this table. Delete
  if still unused after the API work lands.
