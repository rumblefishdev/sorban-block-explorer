---
id: '0025'
title: 'XDR parsing: operation extraction and INVOKE_HOST_FUNCTION details'
type: FEATURE
status: active
related_adr: ['0004']
related_tasks: ['0002', '0024', '0017']
tags: [priority-high, effort-medium, layer-indexing, rust]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: active
    who: FilipDz
    note: 'Promoted to active'
  - date: 2026-03-30
    status: backlog
    who: fmazur
    note: 'Scope changed to Rust-only per ADR 0004.'
---

# XDR parsing: operation extraction and INVOKE_HOST_FUNCTION details

## Summary

Implement operation-level extraction from parsed transactions, handling all Stellar operation types with special attention to INVOKE_HOST_FUNCTION. Each operation is stored with its type and a JSONB details payload. For Soroban host function invocations, the parser additionally extracts contractId, functionName, functionArgs (ScVal decoded), and returnValue (ScVal decoded). Operations are written to the partitioned operations table.

## Status: Backlog

**Current state:** Not started. Depends on task 0024 (LedgerCloseMeta deserialization) for parsed transaction data and task 0017 (operations table schema).

## Context

Each transaction in a ledger contains one or more operations. The explorer needs per-operation structured data so transaction detail pages can render human-readable operation lists without reparsing XDR on every request.

The operations table uses JSONB for the `details` column because operation-specific fields vary heavily by operation type. For INVOKE_HOST_FUNCTION operations, richer extraction is needed to support Soroban contract interaction views.

The operations table is partitioned by `transaction_id`. All inserts must be partition-aware, meaning the transaction's surrogate id (from task 0024) must be known before inserting operations.

### Important Boundary

This task writes to `operations.details` JSONB only. The `soroban_invocations` rows (flat invocation records with full metadata) are owned by task 0026. This task extracts operation-level data; task 0026 extracts invocation-level data from result_meta_xdr.

### Source Code Location

- `apps/indexer/src/parsers/operations/`

## Implementation Plan

### Step 1: Operation Type Extraction

For each transaction (from task 0024 output), iterate over all operations in the TransactionEnvelope. Extract:

- Operation `type` as a string (e.g., "CREATE_ACCOUNT", "PAYMENT", "INVOKE_HOST_FUNCTION", etc.)
- Operation source account (if different from transaction source)

### Step 2: Operation Details JSONB Construction

For each operation type, build a structured JSONB `details` object containing type-specific decoded fields. Examples:

- PAYMENT: destination, asset, amount
- CREATE_ACCOUNT: destination, startingBalance
- CHANGE_TRUST: asset, limit
- PATH_PAYMENT_STRICT_RECEIVE/SEND: sendAsset, destAsset, sendAmount, destAmount, path
- MANAGE_SELL_OFFER / MANAGE_BUY_OFFER: selling, buying, amount, price, offerId
- And all other classic operation types

### Step 3: INVOKE_HOST_FUNCTION Detail Extraction

For INVOKE_HOST_FUNCTION operations, extract additional fields into the details JSONB:

- `contractId`: the target contract address
- `functionName`: the invoked function name
- `functionArgs`: array of ScVal-decoded arguments. Each ScVal is decoded to its typed representation (integer, string, address, bytes, map, list) using the Rust ScVal decoding in the Ledger Processor (per ADR 0004).
- `returnValue`: the ScVal-decoded return value from SorobanTransactionMeta

### Step 4: Unknown Operation Type Handling

When an operation type is not recognized by the current SDK version:

- Store the operation with `type: 'unknown'`
- Log an alarm-level message indicating an unknown operation type was encountered, suggesting SDK update is needed
- Do NOT drop the operation -- it must remain visible in the explorer
- Include whatever raw data is available in the details JSONB

### Step 5: Partition-Aware Insertion

Write operation rows to the operations table. Since the table is partitioned by `transaction_id`:

- The parent transaction must already be persisted (or at least have its surrogate id assigned) before operations are inserted
- Batch insert all operations for a given transaction in a single statement where possible
- Maintain operation ordering within a transaction (operation index)

## Acceptance Criteria

- [ ] All standard Stellar operation types are extracted with appropriate type-specific details JSONB
- [ ] INVOKE_HOST_FUNCTION operations include contractId, functionName, functionArgs (ScVal decoded), and returnValue (ScVal decoded) in details
- [ ] ScVal decoding handles integer, string, address, bytes, map, and list types
- [ ] Unknown operation types are stored as `{ type: 'unknown' }` with alarm logging, not dropped
- [ ] Operations are inserted into the partitioned operations table with correct transaction_id FK
- [ ] Operation ordering within a transaction is preserved
- [ ] Batch insertion is used for efficiency
- [ ] Unit tests cover each major operation type, INVOKE_HOST_FUNCTION detail extraction, unknown type handling, and ScVal decoding
- [ ] ON DELETE CASCADE from transactions properly cleans up operation rows

## Notes

- The GIN index on `operations.details` supports JSONB queries but may need careful consideration for insert performance at scale.
- Operation extraction runs as step 2 in the parser orchestration: 0024 (ledger+tx) -> 0025 (operations) -> 0026 (soroban events/invocations) -> 0027 (entry changes).
- The soroban_invocations table (task 0026) provides a separate, richer view of contract calls with caller_account, successful status, and ledger_sequence. This task's INVOKE_HOST_FUNCTION details are the operation-level view only.
