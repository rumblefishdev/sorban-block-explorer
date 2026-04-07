---
id: '0029'
title: 'Indexer: Ledger Processor Lambda handler'
type: FEATURE
status: done
related_adr: ['0004', '0005']
related_tasks: ['0024', '0025', '0026', '0027', '0028', '0092']
tags: [priority-high, effort-medium, layer-indexing, rust]
milestone: 1
links:
  - docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: apps/indexer/ → crates/indexer/'
  - date: 2026-04-03
    status: active
    who: FilipDz
    note: 'Activated for implementation'
  - date: 2026-04-07
    status: done
    who: FilipDz
    note: >
      Implemented Lambda handler with full pipeline orchestration.
      4 new files (handler/mod.rs, process.rs, convert.rs, persist.rs),
      refactored db crate to accept Acquire trait for transaction support.
      All 4 parsing stages wired in correct order. Atomic DB transaction
      per ledger. Full workspace builds cleanly.
---

# Indexer: Ledger Processor Lambda handler

## Summary

Implement the top-level Lambda handler for the Ledger Processor that receives S3 PutObject events, downloads and decompresses XDR files, orchestrates the four parsing stages (0024 -> 0025 -> 0026 -> 0027), and wraps all writes for a single ledger in one atomic database transaction. This is the entry point and orchestration layer for the entire indexing pipeline.

## Context

The Ledger Processor Lambda is triggered by S3 PutObject events when Galexie (live) or the backfill task writes a new LedgerCloseMeta XDR file to the `stellar-ledger-data` S3 bucket. It is the only worker that turns raw ledger-close artifacts into first-class explorer records.

## Acceptance Criteria

- [x] Lambda handler correctly parses S3 PutObject events and extracts bucket/key
- [x] S3 key pattern validation rejects non-matching keys without infinite retry
- [x] XDR download and zstd decompression work correctly
- [x] Parser stages are orchestrated in correct order: 0024 -> 0025 -> 0026 -> 0027
- [x] All writes for a single ledger are wrapped in one atomic database transaction
- [x] On ledger-level failure, the transaction is rolled back and Lambda retry handles it
- [x] On transaction-level failure, raw XDR is stored with parse_error=true, remaining transactions continue, and the ledger commits
- [x] Error logging includes full context (ledger sequence, S3 key, failing stage, error details)
- [x] All database connections go through RDS Proxy
- [ ] Handler completes within the target latency of <10 seconds for typical ledgers (deferred — requires live environment testing)
- [ ] Integration test covers the full pipeline from S3 event to database verification (deferred to follow-up task)
- [ ] End-to-end latency from S3 object creation to DB write is under 10 seconds under normal conditions (deferred — requires live environment)

## Implementation Notes

### Files created

- `crates/indexer/src/handler/mod.rs` — Lambda entry point: S3 event types, S3 download, dispatch per-ledger
- `crates/indexer/src/handler/process.rs` — Per-ledger orchestration across all 4 stages, atomic DB tx
- `crates/indexer/src/handler/convert.rs` — Extracted\* → domain type conversion (17 functions)
- `crates/indexer/src/handler/persist.rs` — 13-step persistence within DB transaction

### Files modified

- `crates/indexer/Cargo.toml` — Added lambda_runtime, aws-sdk-s3, stellar-xdr, sqlx, etc.
- `crates/indexer/src/main.rs` — Lambda runtime init, Secrets Manager / DATABASE_URL resolution
- `crates/db/src/persistence.rs` — Refactored from `&PgPool` to `impl Acquire<'_, Database = Postgres>`
- `crates/db/src/soroban.rs` — Same Acquire refactor
- `crates/xdr-parser/src/lib.rs` — Made `envelope` module public

## Design Decisions

### From Plan

1. **Atomic DB transaction per ledger**: All writes (ledger, transactions, operations, events, invocations, derived state) wrapped in a single `pool.begin()` / `commit()`. Partial ledgers never visible.

2. **Parser stage ordering**: 0024 → 0025 → 0026 → 0027 strictly sequential per transaction. Each stage depends on previous output.

3. **Non-matching S3 keys logged and skipped**: Returns success to avoid Lambda infinite retry loops.

4. **DATABASE_URL env var with Secrets Manager fallback**: Local dev uses env var directly; deployed Lambda resolves credentials from AWS Secrets Manager via `DB_SECRET_ARN` + `RDS_ENDPOINT`.

### Emerged

5. **Refactored db crate to use `sqlx::Acquire` trait**: The db functions previously took `&PgPool` which prevented use within a transaction. Changed all persistence/soroban functions to accept `impl Acquire<'_, Database = Postgres>` — this supports both `&PgPool` (existing callers, tests) and `&mut Transaction` (handler) without API breakage.

6. **Made xdr-parser `envelope` module public**: It was `pub(crate)` but the handler needs `extract_envelopes()` and `inner_transaction()` to get per-transaction XDR references for stages 0025-0027.

7. **Contract interface metadata deferred**: `ExtractedContractInterface` only carries `wasm_hash`, not `contract_id`. Storing it correctly requires a wasm_hash→contract_id join that doesn't exist yet. Marked as TODO in persist.rs.

8. **Multiple S3 records processed independently**: If one record fails, the handler returns an error immediately (letting Lambda retry). Other records in the same event are not processed. This matches Lambda's retry semantics.

## Issues Encountered

- **`sqlx::Transaction` doesn't implement `AsRef<PgPool>`**: Initial attempt to extract a pool reference from the transaction failed. Resolved by refactoring all db functions to use the `Acquire` trait pattern with `executor.acquire().await?`.

- **`stellar_xdr` types needed in indexer**: The process module needs `LedgerCloseMeta` and `TransactionMeta` types. Added `stellar-xdr` as a direct dependency rather than re-exporting from xdr-parser.

- **`LedgerCloseMetaBatch` field name**: The field is `ledger_close_metas` (plural), not `ledger_close_meta`. Caught at compile time.

## Future Work

- Integration test for full pipeline (S3 event → DB verification) — requires test fixtures with real XDR data
- Latency benchmarking in deployed environment
- Contract interface metadata storage (needs wasm_hash → contract_id mapping)
