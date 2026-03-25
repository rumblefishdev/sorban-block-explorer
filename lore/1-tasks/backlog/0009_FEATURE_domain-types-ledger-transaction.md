---
id: '0009'
title: 'Domain types: ledger and transaction models'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0008']
tags: [priority-high, effort-small, layer-domain]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Domain types: ledger and transaction models

## Summary

Define the shared TypeScript domain types for ledgers, transactions, operations, pagination, and API response shapes. These types live in `libs/domain` and are consumed by both `apps/api` and `apps/indexer`. They mirror the PostgreSQL schema defined in task 0008 and the API response contracts from the backend overview.

## Status: Backlog

**Current state:** Not started. Depends on DB schema task 0008 for final column-level decisions.

## Context

The block explorer needs a single source of truth for domain types that both the ingestion pipeline and the backend API share. Without shared types, the indexer and API risk diverging on field names, nullability, and response shapes. This task captures every field from the DDL and the documented API response contracts for ledgers, transactions, operations, and pagination.

### Ledger fields (from DDL)

| Field             | DB Type                     | Notes                                      |
| ----------------- | --------------------------- | ------------------------------------------ |
| sequence          | BIGINT PRIMARY KEY          | Natural stable PK for ledger navigation    |
| hash              | VARCHAR(64) UNIQUE NOT NULL | Unique but not primary explorer lookup key |
| closed_at         | TIMESTAMPTZ NOT NULL        | Supports recent-history ordering           |
| protocol_version  | INT NOT NULL                |                                            |
| transaction_count | INT NOT NULL                |                                            |
| base_fee          | BIGINT NOT NULL             |                                            |

Index: `idx_closed_at (closed_at DESC)`.

### Transaction fields (from DDL)

| Field           | DB Type                     | Notes                                      |
| --------------- | --------------------------- | ------------------------------------------ |
| id              | BIGSERIAL PRIMARY KEY       | Internal surrogate for child FKs           |
| hash            | VARCHAR(64) UNIQUE NOT NULL | Main public lookup key                     |
| ledger_sequence | BIGINT FK to ledgers        | Links tx back to ledger timeline           |
| source_account  | VARCHAR(56) NOT NULL        |                                            |
| fee_charged     | BIGINT NOT NULL             |                                            |
| successful      | BOOLEAN NOT NULL            |                                            |
| result_code     | VARCHAR(50) nullable        |                                            |
| envelope_xdr    | TEXT NOT NULL               | Raw XDR for advanced decode/debug          |
| result_xdr      | TEXT NOT NULL               | Raw XDR for advanced decode/debug          |
| result_meta_xdr | TEXT nullable               | For advanced decode/debug                  |
| memo_type       | VARCHAR(20) nullable        |                                            |
| memo            | TEXT nullable               |                                            |
| created_at      | TIMESTAMPTZ NOT NULL        |                                            |
| parse_error     | BOOLEAN DEFAULT FALSE       | Allows partial retention when decode fails |
| operation_tree  | JSONB nullable              | Decoded invocation hierarchy               |

Indexes: `idx_hash (hash)`, `idx_source (source_account, created_at DESC)`, `idx_ledger (ledger_sequence)`.

### Operation fields (from DDL)

| Field          | DB Type               | Notes                        |
| -------------- | --------------------- | ---------------------------- |
| id             | BIGSERIAL PRIMARY KEY |                              |
| transaction_id | BIGINT FK CASCADE     | ON DELETE CASCADE            |
| type           | VARCHAR(50) NOT NULL  |                              |
| details        | JSONB NOT NULL        | Type-specific decoded fields |

For `INVOKE_HOST_FUNCTION`, `details` includes: `{ contractId, functionName, functionArgs, returnValue }`.

### Pagination types

**Request:**

```typescript
{
  cursor: string | null;
  limit: number;
}
```

**Response:**

```typescript
{
  data: T[];
  nextCursor: string | null;
  hasMore: boolean;
}
```

Cursors are opaque to clients. No total counts -- ordering is deterministic for stable browsing.

### API response types

**TransactionSummary** (list views): hash, ledgerSequence, sourceAccount, operationType, successful, feeCharged, createdAt.

**TransactionDetail** (full view): all summary fields plus operations array, events, XDR fields (envelope_xdr, result_xdr), and operation_tree.

**LedgerSummary**: sequence, hash, closedAt, protocolVersion, transactionCount, baseFee.

**LedgerDetail**: all summary fields plus linked transactions array.

## Implementation Plan

### Step 1: Define Ledger domain types

Create `Ledger`, `LedgerSummary`, and `LedgerDetail` types in `libs/domain` with all fields from the DDL and API response shapes.

### Step 2: Define Transaction domain types

Create `Transaction`, `TransactionSummary`, and `TransactionDetail` types with all fields listed above. `TransactionDetail` includes operations, events, XDR, and operation_tree.

### Step 3: Define Operation domain type

Create `Operation` type with id, transactionId, type, and details JSONB. Define the `InvokeHostFunctionDetails` sub-type for contractId, functionName, functionArgs, returnValue.

### Step 4: Define Pagination types

Create generic `PaginationRequest` and `PaginatedResponse<T>` types with cursor, limit, data, nextCursor, hasMore.

### Step 5: Export and verify

Export all types from `libs/domain` barrel file. Verify that the types compile and match the DDL field list exactly.

## Acceptance Criteria

- [ ] `Ledger`, `LedgerSummary`, `LedgerDetail` types defined with all DDL fields
- [ ] `Transaction`, `TransactionSummary`, `TransactionDetail` types defined with all DDL fields
- [ ] `Operation` type defined with INVOKE_HOST_FUNCTION details sub-type
- [ ] `PaginationRequest` and `PaginatedResponse<T>` generic types defined
- [ ] All types exported from `libs/domain` barrel
- [ ] Types compile without errors
- [ ] Field names, nullability, and types match the DDL and API response contracts

## Notes

- `id` on transactions is an internal surrogate key; public lookups use `hash`.
- `parse_error` flag supports partial-record retention when XDR decode fails (see task 0014).
- `operation_tree` is JSONB decoded at ingestion time, not reparsed per request.
- Pagination uses opaque cursors; no expensive total counts.
