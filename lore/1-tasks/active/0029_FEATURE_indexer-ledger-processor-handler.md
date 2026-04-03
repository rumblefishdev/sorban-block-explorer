---
id: '0029'
title: 'Indexer: Ledger Processor Lambda handler'
type: FEATURE
status: active
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
---

# Indexer: Ledger Processor Lambda handler

## Summary

Implement the top-level Lambda handler for the Ledger Processor that receives S3 PutObject events, downloads and decompresses XDR files, orchestrates the four parsing stages (0024 -> 0025 -> 0026 -> 0027), and wraps all writes for a single ledger in one atomic database transaction. This is the entry point and orchestration layer for the entire indexing pipeline.

## Status: Backlog

**Current state:** Not started. Depends on all four parser tasks (0024-0027) and idempotent write logic (0028).

## Context

The Ledger Processor Lambda is triggered by S3 PutObject events when Galexie (live) or the backfill task writes a new LedgerCloseMeta XDR file to the `stellar-ledger-data` S3 bucket. It is the only worker that turns raw ledger-close artifacts into first-class explorer records.

The handler must orchestrate all parsing stages in order, wrap all resulting database writes in a single transaction per ledger, and handle errors gracefully to preserve explorer continuity.

### S3 Key Pattern

Files arrive at: `stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd`

### Target Latency

Less than 10 seconds from ledger close to database write.

### Retry Model

The Lambda is auto-retried by S3 event notification on failure. Failed files remain in S3 for manual replay. A DLQ (SQS) captures exhausted retries (configured in CDK task 0033).

### Open-Source Redeployability

No hidden dependencies on internal services or external explorer APIs. The entire pipeline is self-contained.

### Source Code Location

- `crates/indexer/src/handler/`

## Implementation Plan

### Step 1: S3 Event Parsing

Parse the incoming Lambda event to extract the S3 bucket and key. Validate that the key matches the expected pattern: `stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd`. Reject events that do not match (log and return success to avoid infinite retry).

### Step 2: Download and Decompress

Download the XDR file from S3 and decompress using zstd. This step is shared with task 0024's implementation but owned by the handler as the entry point.

### Step 3: Parser Orchestration

Execute parsing stages in sequence:

1. **Task 0024**: LedgerCloseMeta deserialization, ledger header extraction, transaction extraction
2. **Task 0025**: Operation extraction per transaction
3. **Task 0026**: Soroban event extraction, invocation tree decoding, contract interface extraction
4. **Task 0027**: LedgerEntryChanges extraction (contracts, accounts, tokens, NFTs, pools)

Each stage receives output from the previous stage. The orchestrator collects all resulting database operations.

### Step 4: Atomic Database Transaction

Wrap ALL writes for a single ledger in ONE database transaction:

- Ledger row
- Transaction rows (with surrogate ids assigned)
- Operation rows (using transaction surrogate ids)
- Soroban invocation rows
- Soroban event rows
- Derived-state upserts (contracts, accounts, tokens, NFTs, pools, snapshots)

Commit or rollback atomically. A partial ledger must never be visible in the database.

Use RDS Proxy for all database connections (never connect directly to RDS from Lambda).

### Step 5: Error Handling

On parse failure at any stage:

- Log the error with full context: ledger sequence, S3 key, stage that failed, error message, stack trace
- If the failure is at the transaction level (not the ledger level), store raw XDR for the failing transaction, mark `parse_error = true`, and continue processing remaining transactions
- If the failure is at the ledger level (decompression failure, top-level deserialization failure), log and let Lambda retry handle it
- Commit partial data only when individual transaction parsing fails (the ledger and all successfully-parsed transactions are committed)

### Step 6: Schema Migration Awareness

Schema migrations must be applied before deploying new Lambda code. This is enforced in the CI/CD pipeline (task 0039). The handler itself does not run migrations but should fail clearly if the schema is incompatible (e.g., missing table or column).

## Acceptance Criteria

- [ ] Lambda handler correctly parses S3 PutObject events and extracts bucket/key
- [ ] S3 key pattern validation rejects non-matching keys without infinite retry
- [ ] XDR download and zstd decompression work correctly
- [ ] Parser stages are orchestrated in correct order: 0024 -> 0025 -> 0026 -> 0027
- [ ] All writes for a single ledger are wrapped in one atomic database transaction
- [ ] On ledger-level failure, the transaction is rolled back and Lambda retry handles it
- [ ] On transaction-level failure, raw XDR is stored with parse_error=true, remaining transactions continue, and the ledger commits
- [ ] Error logging includes full context (ledger sequence, S3 key, failing stage, error details)
- [ ] All database connections go through RDS Proxy
- [ ] Handler completes within the target latency of <10 seconds for typical ledgers
- [ ] Integration test covers the full pipeline from S3 event to database verification
- [ ] End-to-end latency from S3 object creation to DB write is under 10 seconds under normal conditions

## Notes

- The handler must be stateless between invocations. All state is in the database.
- Protocol upgrades require updating the `stellar-xdr` Rust crate (per ADR 0004). Exhaustive `match` on XDR enums ensures new variants cause compile errors. Upgrades are infrequent and announced in advance.
- The S3 event may contain multiple records (multiple files). Each should be processed independently. If one fails, others should still succeed.
- Connection pooling via RDS Proxy is critical under burst Lambda execution. The handler should not hold connections longer than necessary.
- The ordering of parser stages matters: operations (0025) need transaction surrogate ids from 0024, invocations/events (0026) need transaction ids, and entry changes (0027) need the full parsed context.
