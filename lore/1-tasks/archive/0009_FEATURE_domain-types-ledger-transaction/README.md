---
id: '0009'
title: 'Domain types: ledger and transaction models'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0008']
tags: [priority-high, effort-small, layer-domain]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-27
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active'
  - date: 2026-03-27
    status: completed
    who: stkrolikiewicz
    note: >
      Implemented 13 types in libs/domain/src/index.ts.
      All 7 acceptance criteria met. PR #27.
      Key decisions: BigIntString for BIGINT columns,
      OperationType open union, readonly on response arrays.
---

# Domain types: ledger and transaction models

## Summary

Define the shared TypeScript domain types for ledgers, transactions, operations, pagination, and API response shapes. These types live in `libs/domain` and are consumed by both `apps/api` and `apps/indexer`. They mirror the PostgreSQL schema defined in tasks 0016-0020 and the API response contracts from the backend overview.

## Status: Completed

## Acceptance Criteria

- [x] `Ledger`, `LedgerSummary`, `LedgerDetail` types defined with all DDL fields
- [x] `Transaction`, `TransactionSummary`, `TransactionDetail` types defined with all DDL fields
- [x] `Operation` type defined with INVOKE_HOST_FUNCTION details sub-type
- [x] `PaginationRequest` and `PaginatedResponse<T>` generic types defined
- [x] All types exported from `libs/domain` barrel
- [x] Types compile without errors
- [x] Field names, nullability, and types match the DDL and API response contracts

## Implementation Notes

**File:** `libs/domain/src/index.ts` — 13 new exported types added to existing single-file barrel.

**Types added:**

- `Ledger`, `LedgerPointer` (Pick), `LedgerSummary` (alias), `LedgerDetail`
- `Transaction`, `TransactionPointer` (Pick), `TransactionSummary`, `TransactionDetail`
- `Operation`, `OperationType` (27 Stellar ops + open catch-all), `InvokeHostFunctionDetails`
- `PaginationRequest`, `PaginatedResponse<T>`

## Design Decisions

### From Plan

1. **`BigIntString` for all BIGINT/BIGSERIAL columns**: Aligns with convention established by task 0010. Includes ids, sequences, and fees.

2. **`LedgerSummary = Ledger` type alias**: All 6 DDL fields match the API summary spec. Separate name preserves semantic distinction (entity vs API shape).

3. **`TransactionSummary` is standalone, not `Pick<Transaction>`**: Contains `operationType` — a derived field not in the Transaction entity.

4. **`TransactionDetail extends TransactionSummary`**: Guarantees every summary field exists on detail. Adds XDR, operations, events, operationTree.

5. **`LedgerPointer` and `TransactionPointer` as `Pick<>` derivations**: Replaces original standalone interfaces. Now structurally tied to canonical entity types.

### Emerged

6. **Stayed in single `index.ts` instead of splitting into modules**: Plan called for `ledger.ts`, `transaction.ts`, etc. Task 0010 was completed first and kept single-file convention. Followed existing pattern to avoid scope creep.

7. **`InvokeHostFunctionDetails.returnValue: ScVal | null`**: Made nullable (plan had non-nullable). Research task 0002 showed return_value may be absent for void functions or failed invocations.

8. **`readonly` on all response arrays**: Applied after PR review. `operations`, `events`, `transactions`, `data` all marked `readonly` for consistency with 0010's JSONB array convention.

9. **`Operation.type: OperationType` instead of `string`**: Applied after PR review. The `(string & {})` catch-all preserves compatibility while giving IDE autocomplete for known operation types.

10. **`JsonValue` for JSONB fields, `ScVal` for Soroban values**: Aligned with 0010 primitives instead of using `unknown` or `Record<string, unknown>`.

## Issues Encountered

- **Nx socket path too long**: `@nx/vite/plugin` failed with "socket exceeds maximum length" on this repo path. Workaround: `NX_DAEMON=false NX_SOCKET_DIR=/tmp/nx-tmp`. Affects commit/push hooks.

- **Task 0010 merged to develop mid-implementation**: Required reworking types from `number`/`unknown` to `BigIntString`/`JsonValue`/`ScVal` and abandoning the multi-file split.

## Future Work

- `SorobanInvocation.functionArgs` in 0010 types should be `readonly ScVal[]`, not `ScVal | null` (research 0002 confirms it's always an array)

## Notes

- `id` on transactions is an internal surrogate key; public lookups use `hash`.
- `parse_error` flag supports partial-record retention when XDR decode fails (see task 0014).
- `operation_tree` is JSONB decoded at ingestion time, not reparsed per request.
- Pagination uses opaque cursors; no expensive total counts.
