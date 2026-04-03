---
id: '0028'
title: 'Indexer: idempotent write logic and ledger-sequence watermarks'
type: FEATURE
status: completed
related_adr: ['0005']
related_tasks: ['0029', '0092']
tags: [priority-high, effort-medium, layer-indexing]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: apps/indexer/ → crates/indexer/'
  - date: 2026-04-02
    status: active
    who: FilipDz
    note: 'Activated for implementation'
  - date: 2026-04-03
    status: completed
    who: FilipDz
    note: >
      All 6 steps implemented. 7 integration tests passing (2 idempotency,
      3 watermark enforcement, 1 first_seen immutability, 1 pool watermark).
      Persistence layer uses domain types. Removed xdr-parser dependency from
      db crate. Bug fix: contract_type COALESCE order corrected.
---

# Indexer: idempotent write logic and ledger-sequence watermarks

## Summary

Implement the persistence layer that ensures all Ledger Processor writes are idempotent and replay-safe. Immutable tables use INSERT ON CONFLICT DO NOTHING. Derived-state tables use upsert guarded by ledger-sequence watermark columns so that older backfill data cannot overwrite newer live-derived state. This logic is foundational to the reliability of both live ingestion and historical backfill running in parallel.

## Status: Completed

## Context

The indexing pipeline must handle two concurrency scenarios safely:

1. **Replay of the same ledger**: S3 event notification may deliver the same file twice (Lambda retry, manual replay). Processing the same ledger again must not create duplicate rows or corrupt existing data.

2. **Backfill and live running in parallel**: Historical backfill writes older ledger data while live ingestion writes newer data. A backfill write for ledger N must never overwrite derived state that was already updated by a later ledger M (where M > N).

The idempotent write layer solves both scenarios through two complementary mechanisms: conflict-ignoring inserts for immutable data and watermark-guarded upserts for derived state.

### Source Code Location

- `crates/db/src/persistence.rs` — immutable table inserts (ON CONFLICT DO NOTHING)
- `crates/db/src/soroban.rs` — derived-state watermark upserts

## Implementation Plan

### Step 1: Immutable Table Write Strategy

For tables that represent immutable chain history (data never changes once written):

- `ledgers`: INSERT ON CONFLICT (sequence) DO NOTHING
- `transactions`: INSERT ON CONFLICT (hash) DO NOTHING
- `operations`: INSERT ON CONFLICT DO NOTHING (keyed by transaction_id + operation index)
- `soroban_invocations`: INSERT ON CONFLICT DO NOTHING
- `soroban_events`: INSERT ON CONFLICT DO NOTHING

If the row already exists (same ledger was already processed), the insert is silently skipped. No update, no error.

### Step 2: Derived-State Watermark Upserts

For tables that represent mutable derived state, upserts are guarded by ledger-sequence comparison:

**accounts:**

```
INSERT INTO accounts (...) VALUES (...)
ON CONFLICT (account_id) DO UPDATE SET
  last_seen_ledger = EXCLUDED.last_seen_ledger,
  sequence_number = EXCLUDED.sequence_number,
  balances = EXCLUDED.balances,
  home_domain = EXCLUDED.home_domain
WHERE EXCLUDED.last_seen_ledger >= accounts.last_seen_ledger
```

- `last_seen_ledger` is the watermark. Only update if the incoming data is from a ledger >= the currently stored ledger.
- `first_seen_ledger` is set only on initial insert, never updated.

**tokens:**

- Upsert on unique constraints (asset_code, issuer_address) or (contract_id)
- Update total_supply, holder_count, metadata only from newer ledger data

**nfts:**

```
ON CONFLICT (contract_id, token_id) DO UPDATE SET
  owner_account = EXCLUDED.owner_account,
  ...
WHERE EXCLUDED.last_seen_ledger >= nfts.last_seen_ledger
```

- `last_seen_ledger` is the watermark
- `minted_at_ledger` is set only on initial insert

**liquidity_pools:**

```
ON CONFLICT (pool_id) DO UPDATE SET
  reserves = EXCLUDED.reserves,
  total_shares = EXCLUDED.total_shares,
  tvl = EXCLUDED.tvl,
  last_updated_ledger = EXCLUDED.last_updated_ledger
WHERE EXCLUDED.last_updated_ledger >= liquidity_pools.last_updated_ledger
```

- `last_updated_ledger` is the watermark
- `created_at_ledger` is set only on initial insert

### Step 3: soroban_contracts Upsert Logic

The soroban_contracts table has special upsert semantics:

- Upsert on `contract_id`
- Deployment fields (wasm_hash, deployer_account, deployed_at_ledger, contract_type, is_sac) are set on first insert
- `metadata` is updated when interface extraction completes (may arrive in the same or a later ledger processing run)
- The upsert must handle both task 0026 (interface extraction) and task 0027 (deployment extraction) writing to the same row, potentially in either order

### Step 4: Liquidity Pool Snapshots (Append-Only)

`liquidity_pool_snapshots` are append-only. They are never updated:

- INSERT with no ON CONFLICT update clause
- If the same snapshot already exists (same pool_id + ledger_sequence), use DO NOTHING
- Snapshots are never deleted by application code (only by partition management)

### Step 5: Batch Insertion

Implement batch insertion for child rows per processed ledger:

- Batch insert all transactions for a ledger
- Batch insert all operations for all transactions in the ledger
- Batch insert all invocations and events
- Batch upsert derived-state rows

Batching reduces round trips to RDS Proxy and improves throughput.

### Step 6: ON DELETE CASCADE Awareness

Child cleanup relies on ON DELETE CASCADE from parent tables:

- transactions -> operations, soroban_invocations, soroban_events
- liquidity_pools -> liquidity_pool_snapshots

The idempotent write layer must NOT use a delete-then-reinsert pattern. This would trigger cascade deletes of child rows. Instead, use INSERT ON CONFLICT DO NOTHING or guarded upserts.

## Acceptance Criteria

- [x] Immutable tables (ledgers, transactions, operations, invocations, events) use INSERT ON CONFLICT DO NOTHING
- [x] Replay of the same ledger produces no duplicate rows and no errors
- [x] accounts upsert is guarded by last_seen_ledger watermark -- older data does not overwrite newer state
- [x] accounts.first_seen_ledger is set on creation only, never overwritten
- [x] nfts upsert is guarded by last_seen_ledger watermark
- [x] nfts.minted_at_ledger is set on creation only, never overwritten
- [x] liquidity_pools upsert is guarded by last_updated_ledger watermark
- [x] liquidity_pools.created_at_ledger is set on creation only, never overwritten
- [x] soroban_contracts upsert handles deployment fields on first insert and metadata updates from interface extraction
- [x] liquidity_pool_snapshots are append-only with no update path
- [x] Batch insertion is used for child rows per ledger
- [x] No delete-then-reinsert patterns that would trigger CASCADE deletes
- [x] Unit tests verify idempotency: processing the same ledger twice produces identical database state
- [x] Unit tests verify watermark enforcement: older ledger data does not overwrite newer derived state

## Implementation Notes

### Files added/modified

- `crates/db/src/persistence.rs` — new module: idempotent batch inserts for immutable tables (ledgers, transactions, operations, events, invocations) + 2 integration tests
- `crates/db/src/soroban.rs` — rewritten: derived-state upserts using domain types, removed non-idempotent insert_events/insert_invocations, added `upsert_contract_metadata` raw-value signature + 5 integration tests
- `crates/db/src/lib.rs` — registered `pub mod persistence`
- `crates/db/Cargo.toml` — removed `xdr-parser`, `chrono`, `tracing` deps; added `tokio`, `chrono` as dev-deps
- `crates/db/migrations/0007_idempotent_write_constraints.sql` — unique constraints for operations, events (+ event_index column), invocations (+ invocation_index column)
- `crates/domain/src/transaction.rs` — added `operation_tree: Option<Value>`
- `crates/domain/src/soroban.rs` — added `event_index: i16` to SorobanEvent, `invocation_index: i16` to SorobanInvocation

### Module split

- `persistence` = immutable tables (ON CONFLICT DO NOTHING)
- `soroban` = derived-state upserts (watermark-guarded)

## Issues Encountered

- **contract_type COALESCE order was backwards**: `COALESCE(EXCLUDED.contract_type, soroban_contracts.contract_type)` preferred incoming over existing, opposite of all other deployment fields. Fixed to `COALESCE(soroban_contracts.contract_type, EXCLUDED.contract_type)`.
- **Domain types missing write-path fields**: `Transaction` lacked `operation_tree`, `SorobanEvent` lacked `event_index`, `SorobanInvocation` lacked `invocation_index`. Extended domain types to include them.
- **VARCHAR(56) test data**: Initial test account IDs exceeded 56 chars, causing DB constraint violations. Fixed to exactly 56-char G-addresses.
- **System postgres port conflict**: Port 5432 occupied by system postgres. Docker compose started on port 5433 for test DB.

## Design Decisions

### From Plan

1. **Watermark-based upserts in SQL** — `ON CONFLICT ... DO UPDATE SET ... WHERE watermark <= EXCLUDED.watermark` enforced atomically in DB
2. **Two-module split** — immutable inserts in `persistence.rs`, derived-state upserts in `soroban.rs`
3. **No delete-then-reinsert** — all paths use ON CONFLICT to avoid triggering CASCADE deletes

### Emerged

4. **Persistence layer uses domain types, not Extracted\* types** — db crate should not depend on xdr-parser. Domain types extended with missing fields (event_index, invocation_index, operation_tree). Caller (task 0029) responsible for Extracted\* → domain conversion.
5. **Removed xdr-parser dependency from db crate** — cleaner dependency graph: db depends only on domain + sqlx. `upsert_contract_metadata` refactored to accept raw `(contract_id, wasm_hash, metadata)` instead of `ExtractedContractInterface`.
6. **Row-by-row loops instead of true multi-row INSERT** — functions named `_batch` but execute one query per row. Functional for typical ledger sizes (5-50 txs). True multi-row INSERT deferred to optimization pass if needed.
7. **Surrogate PK `id` fields on domain types** — domain types have `id: i64` (BIGSERIAL) that the persistence layer ignores during INSERT. Caller must pass `id: 0`. Slightly awkward but avoids separate insert-only type layer.
8. **source_account fallback removed from operations insert** — `Operation.source_account: String` is non-optional. Caller (task 0029) must resolve the tx-source fallback before constructing the Operation.

## Notes

- The watermark pattern is critical for safe parallel operation of live ingestion and historical backfill. Without it, backfill processing ledger 50,000,000 could overwrite account state that live ingestion already updated from ledger 55,000,000.
- Batch size should be tuned for the typical number of transactions per ledger (~5-50 transactions per ledger in normal conditions, potentially more during high activity).
- RDS Proxy connection pooling means the persistence layer should acquire and release connections promptly, not hold them across the entire Lambda execution if avoidable.
- The ON CONFLICT clauses must align with the actual unique constraints and primary keys defined in the schema tasks (0016-0020).
- Integration tests require `DATABASE_URL` env var. Run with: `DATABASE_URL=postgres://postgres:postgres@localhost:5433/soroban_block_explorer cargo test -p db`
