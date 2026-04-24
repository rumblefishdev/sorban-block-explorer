---
id: '0163'
title: 'Refactor: operations → operations_appearances (appearance index)'
type: REFACTOR
status: active
related_adr: ['0027', '0033', '0034']
related_tasks: []
tags: [layer-db, layer-indexer, operations, appearance-index, schema-migration]
links: []
history:
  - date: 2026-04-24
    status: active
    who: fmazur
    note: 'Task created and activated'
---

# Refactor: `operations` → `operations_appearances`

## Summary

Slim the `operations` table down to an appearance index (same pattern as
`soroban_events_appearances` / `soroban_invocations_appearances`, ADR 0033/0034)
and rename it to `operations_appearances`. Per-operation detail (amounts, order,
memo, claimants, function args, predicates, …) is re-materialised in the API
from XDR archived in S3 — the DB only records _that_ an operation of a given
shape occurred in a given transaction.

## Status: Active

**Current state:** Task just created. No code changes yet. Design agreed with
owner: drop `transfer_amount` and `application_order`, collapse duplicates with
a `COUNT(*) → amount BIGINT` aggregate (mirrors `soroban_*_appearances.amount`).

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

- [ ] New `operations_appearances` table with the agreed columns + partitions
- [ ] Indexer writes collapsed rows with `amount = COUNT(*)` per identity key
- [ ] Type 14 rowcount drops to ~one per transaction carrying a CCB op
- [ ] Integration test `persist_integration.rs` passes against new schema
- [ ] `cargo check` clean across workspace
- [ ] No references to `transfer_amount` / `application_order` remain
      outside `stellar_archive` (XDR path) and xdr-parser
- [ ] Overview doc updated to new schema

## Notes / Open Questions

- **PK choice.** Appearance tables use the natural composite
  `(contract_id, transaction_id, ledger_sequence, created_at)` as PK and rely
  on `ON CONFLICT DO NOTHING` for replay idempotency. Here the natural key is
  wider (9 columns) and several are NULLable — PG treats each NULL as
  distinct, so natural PK would let NULL-heavy type-14 rows duplicate on
  replay. Options: (a) keep `BIGSERIAL id` + unique constraint with
  `NULLS NOT DISTINCT` on identity cols (PG 15+), (b) coalesce NULLs to a
  sentinel in a unique expression index, (c) accept `id`-only PK and dedupe
  at aggregate time assuming single writer per ledger. Decide during
  implementation and record under `### Emerged` on completion.
- **Index set.** Current indexes are tuned for the old shape; some (e.g.
  `idx_ops_tx`) are probably still right, others (`idx_ops_type`) may lose
  value once `amount` aggregates collapse most traffic. Re-measure.
- **Data preservation.** Local dev DB — drop and re-ingest. No prod yet.
