---
id: '0028'
title: 'Indexer: idempotent write logic and ledger-sequence watermarks'
type: FEATURE
status: backlog
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
---

# Indexer: idempotent write logic and ledger-sequence watermarks

## Summary

Implement the persistence layer that ensures all Ledger Processor writes are idempotent and replay-safe. Immutable tables use INSERT ON CONFLICT DO NOTHING. Derived-state tables use upsert guarded by ledger-sequence watermark columns so that older backfill data cannot overwrite newer live-derived state. This logic is foundational to the reliability of both live ingestion and historical backfill running in parallel.

## Status: Backlog

**Current state:** Not started. Depends on the Ledger Processor handler (task 0029) for integration.

## Context

The indexing pipeline must handle two concurrency scenarios safely:

1. **Replay of the same ledger**: S3 event notification may deliver the same file twice (Lambda retry, manual replay). Processing the same ledger again must not create duplicate rows or corrupt existing data.

2. **Backfill and live running in parallel**: Historical backfill writes older ledger data while live ingestion writes newer data. A backfill write for ledger N must never overwrite derived state that was already updated by a later ledger M (where M > N).

The idempotent write layer solves both scenarios through two complementary mechanisms: conflict-ignoring inserts for immutable data and watermark-guarded upserts for derived state.

### Source Code Location

- `crates/indexer/src/persistence/`

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

- [ ] Immutable tables (ledgers, transactions, operations, invocations, events) use INSERT ON CONFLICT DO NOTHING
- [ ] Replay of the same ledger produces no duplicate rows and no errors
- [ ] accounts upsert is guarded by last_seen_ledger watermark -- older data does not overwrite newer state
- [ ] accounts.first_seen_ledger is set on creation only, never overwritten
- [ ] nfts upsert is guarded by last_seen_ledger watermark
- [ ] nfts.minted_at_ledger is set on creation only, never overwritten
- [ ] liquidity_pools upsert is guarded by last_updated_ledger watermark
- [ ] liquidity_pools.created_at_ledger is set on creation only, never overwritten
- [ ] soroban_contracts upsert handles deployment fields on first insert and metadata updates from interface extraction
- [ ] liquidity_pool_snapshots are append-only with no update path
- [ ] Batch insertion is used for child rows per ledger
- [ ] No delete-then-reinsert patterns that would trigger CASCADE deletes
- [ ] Unit tests verify idempotency: processing the same ledger twice produces identical database state
- [ ] Unit tests verify watermark enforcement: older ledger data does not overwrite newer derived state

## Notes

- The watermark pattern is critical for safe parallel operation of live ingestion and historical backfill. Without it, backfill processing ledger 50,000,000 could overwrite account state that live ingestion already updated from ledger 55,000,000.
- Batch size should be tuned for the typical number of transactions per ledger (~5-50 transactions per ledger in normal conditions, potentially more during high activity).
- RDS Proxy connection pooling means the persistence layer should acquire and release connections promptly, not hold them across the entire Lambda execution if avoidable.
- The ON CONFLICT clauses must align with the actual unique constraints and primary keys defined in the schema tasks (0016-0020).
