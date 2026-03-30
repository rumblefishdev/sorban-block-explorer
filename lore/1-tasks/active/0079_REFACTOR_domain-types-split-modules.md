---
id: '0079'
title: 'Split domain types into per-concern modules'
type: REFACTOR
status: active
related_adr: ['TBD']
related_tasks: ['0009', '0012']
tags: [priority-medium, effort-small, layer-domain]
links:
  - libs/domain/src/index.ts
  - libs/domain/README.md
history:
  - date: 2026-03-27
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-30
    status: active
    who: fmazur
    note: 'Activated task for implementation'
---

# Split domain types into per-concern modules

## Summary

Refactor `libs/domain/src/index.ts` (~370 lines, ~15 type groups) from a single monolithic file into per-concern modules under `libs/domain/src/lib/`. The barrel `index.ts` will re-export everything so that external consumers are unaffected.

## Status: Active

**Current state:** Not started. No blockers — purely internal refactoring with no API surface change.

## Context

As domain types grew (ledger, transaction, operation, pagination, soroban, token, account, NFT, pool, network stats, search), the single `index.ts` became hard to navigate. Splitting into focused modules improves discoverability, makes diffs smaller, and reduces merge conflicts when multiple tasks touch domain types concurrently.

### Proposed module structure

```
libs/domain/src/
├── index.ts                  # barrel re-exports only
└── lib/
    ├── primitives.ts         # JsonValue, ScVal, BigIntString, NumericString
    ├── ledger.ts             # Ledger, LedgerPointer, LedgerSummary, LedgerDetail
    ├── transaction.ts        # Transaction, TransactionPointer, TransactionSummary, TransactionDetail
    ├── operation.ts          # OperationType, InvokeHostFunctionDetails, Operation
    ├── pagination.ts         # PaginationRequest, PaginatedResponse
    ├── soroban.ts            # ContractType, ContractFunction, ContractMetadata, SorobanContract,
    │                         #   EventType, SorobanInvocation, SorobanEvent,
    │                         #   InterpretationType, EventInterpretation
    ├── token.ts              # AssetType, Token
    ├── account.ts            # Account
    ├── nft.ts                # NFT
    ├── pool.ts               # PoolAsset, LiquidityPool, LiquidityPoolSnapshot,
    │                         #   PoolChartInterval, PoolChartDataPoint
    ├── network-stats.ts      # NetworkStats
    └── search.ts             # SearchEntityType, SearchRequest, SearchResultItem, SearchResultGroup
```

## Implementation Plan

### Step 1: Create module files

Move each type group into its own file under `libs/domain/src/lib/`. Each module imports shared primitives from `./primitives` where needed.

### Step 2: Update barrel index.ts

Replace all type definitions with `export * from './lib/<module>';` re-exports so that no external import paths break.

### Step 3: Update libs/domain/README.md

Add a "Module layout" section describing the new file structure and the rationale for the split.

### Step 4: Write ADR

Create an ADR documenting the decision to split domain types into per-concern modules, the alternatives considered (status quo, one-type-per-file), and the chosen granularity.

### Step 5: Verify

Run `pnpm nx lint domain` and `pnpm nx build domain` (or affected) to confirm no breakage.

## Acceptance Criteria

- [ ] Each type group lives in its own file under `libs/domain/src/lib/`
- [ ] `libs/domain/src/index.ts` contains only re-exports
- [ ] All existing imports from `@repo/domain` continue to work without changes
- [ ] `libs/domain/README.md` documents the module layout
- [ ] ADR exists explaining the decision
- [ ] Lint and build pass

## Notes

- This is a zero-consumer-impact refactor — barrel re-exports preserve the public API.
- The split granularity follows the existing comment-delimited sections in `index.ts`.
- If a module would contain only one type (e.g., `Account`), it's still worth splitting for consistency and future growth.
