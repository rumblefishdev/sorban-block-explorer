---
id: '0010'
title: 'Domain types: Soroban models (contract, invocation, event)'
type: FEATURE
status: completed
assignee: fmazur
related_adr: []
related_tasks: ['0010']
tags: [priority-high, effort-small, layer-domain]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-27
    status: active
    who: fmazur
    note: 'Activated task'
  - date: 2026-03-27
    status: completed
    who: fmazur
    note: >
      Implemented all 6 steps. 14 exported types/interfaces added to libs/domain/src/index.ts.
      Key decisions: BigIntString for BIGINT columns, ScVal alias for decoded ScVal,
      JsonValue for JSONB, ContractMetadata with typed functions field,
      readonly arrays, strict DDL nullability alignment.
---

# Domain types: Soroban models (contract, invocation, event)

## Summary

Define the shared TypeScript domain types for Soroban contracts, contract invocations, Soroban events, and event interpretations. These types live in `libs/domain` and are consumed by both `apps/api` and `apps/indexer`. They mirror the PostgreSQL schema for the Soroban-specific tables and the API response contracts.

## Status: Completed

## Context

The block explorer treats Soroban entities as first-class explorer objects. Contracts, invocations, and events each have their own tables, partitioning strategies, and API endpoints. Shared domain types ensure the indexer writes and the API reads agree on field names, nullability, and structure.

### SorobanContract fields (from DDL)

| Field            | DB Type                 | Notes                                     |
| ---------------- | ----------------------- | ----------------------------------------- |
| contractId       | VARCHAR(56) PRIMARY KEY | Public stable identifier                  |
| wasmHash         | VARCHAR(64) nullable    |                                           |
| deployerAccount  | VARCHAR(56) nullable    |                                           |
| deployedAtLedger | BIGINT FK nullable      | References ledgers(sequence)              |
| contractType     | VARCHAR(50)             | 'token', 'dex', 'lending', 'nft', 'other' |
| isSac            | BOOLEAN DEFAULT FALSE   | Stellar Asset Contract flag               |
| metadata         | JSONB                   | May include interface signatures          |
| searchVector     | TSVECTOR GENERATED      | Not in domain type -- DB-only             |

Indexes: `idx_type (contract_type)`, `idx_search (search_vector) USING GIN`.

### ContractType enum

```typescript
'token' | 'dex' | 'lending' | 'nft' | 'other';
```

### ContractFunction type

```typescript
{
  name: string;
  parameters: {
    name: string;
    type: string;
  }
  [];
  returnType: string;
}
```

Extracted from WASM at deployment time and stored in `soroban_contracts.metadata`. Served by `GET /contracts/:contract_id/interface`.

### SorobanInvocation fields (from DDL)

| Field          | DB Type               | Notes                                     |
| -------------- | --------------------- | ----------------------------------------- |
| id             | BIGSERIAL PRIMARY KEY |                                           |
| transactionId  | BIGINT FK CASCADE     | References transactions(id)               |
| contractId     | VARCHAR(56) FK        | References soroban_contracts(contract_id) |
| callerAccount  | VARCHAR(56) nullable  |                                           |
| functionName   | VARCHAR(100) NOT NULL |                                           |
| functionArgs   | JSONB                 | Decoded ScVal                             |
| returnValue    | JSONB                 | Decoded ScVal                             |
| successful     | BOOLEAN NOT NULL      |                                           |
| ledgerSequence | BIGINT NOT NULL       |                                           |
| createdAt      | TIMESTAMPTZ NOT NULL  |                                           |

Monthly partitioned by `created_at`. Indexes: `idx_contract (contract_id, created_at DESC)`, `idx_function (contract_id, function_name)`.

### SorobanEvent fields (from DDL)

| Field          | DB Type               | Notes                                     |
| -------------- | --------------------- | ----------------------------------------- |
| id             | BIGSERIAL PRIMARY KEY |                                           |
| transactionId  | BIGINT FK CASCADE     | References transactions(id)               |
| contractId     | VARCHAR(56) FK        | References soroban_contracts(contract_id) |
| eventType      | VARCHAR(20) NOT NULL  | 'contract', 'system', 'diagnostic'        |
| topics         | JSONB NOT NULL        | Decoded ScVal[] array                     |
| data           | JSONB NOT NULL        | Decoded ScVal                             |
| ledgerSequence | BIGINT NOT NULL       |                                           |
| createdAt      | TIMESTAMPTZ NOT NULL  |                                           |

Monthly partitioned by `created_at`. Indexes: `idx_contract (contract_id, created_at DESC)`, `idx_topics (topics) USING GIN`.

### EventInterpretation fields (from DDL)

| Field              | DB Type               | Notes                              |
| ------------------ | --------------------- | ---------------------------------- |
| id                 | BIGSERIAL PRIMARY KEY |                                    |
| eventId            | BIGINT FK CASCADE     | References soroban_events(id)      |
| interpretationType | VARCHAR(50) NOT NULL  | 'swap', 'transfer', 'mint', 'burn' |
| humanReadable      | TEXT NOT NULL         |                                    |
| structuredData     | JSONB NOT NULL        |                                    |

Written by Event Interpreter Lambda, not primary ingestion. Represents enrichment, not canonical chain truth.

## Implementation Plan

### Step 1: Define ContractType enum and ContractFunction type

Create the `ContractType` union type and `ContractFunction` interface in `libs/domain`.

### Step 2: Define SorobanContract domain type

Create `SorobanContract` type with all DDL fields except `searchVector` (DB-only generated column).

### Step 3: Define SorobanInvocation domain type

Create `SorobanInvocation` type with all fields. Note that `functionArgs` and `returnValue` are decoded ScVal stored as JSONB.

### Step 4: Define SorobanEvent domain type

Create `SorobanEvent` type with all fields. `topics` is a JSONB array of decoded ScVal values. `eventType` is a union of 'contract' | 'system' | 'diagnostic'.

### Step 5: Define EventInterpretation domain type

Create `EventInterpretation` type. Include the `interpretationType` union: 'swap' | 'transfer' | 'mint' | 'burn'.

### Step 6: Export and verify

Export all types from `libs/domain` barrel file. Verify compilation and field alignment with DDL.

## Acceptance Criteria

- [x] `ContractType` union type defined: 'token' | 'dex' | 'lending' | 'nft' | 'other'
- [x] `ContractFunction` type defined with name, parameters (name + type), returnType
- [x] `SorobanContract` type defined with all DDL fields (excluding searchVector)
- [x] `SorobanInvocation` type defined with all DDL fields, JSONB fields typed appropriately
- [x] `SorobanEvent` type defined with all DDL fields, eventType union typed
- [x] `EventInterpretation` type defined with interpretationType union
- [x] All types exported from `libs/domain` barrel
- [x] Types compile without errors

## Notes

- `searchVector` is a generated tsvector column in PostgreSQL and should not appear in the domain type.
- `functionArgs` and `returnValue` on invocations are decoded ScVal stored as JSONB; the domain type should use a generic decoded-ScVal representation (see task 0013 for the ScVal parsing library).
- Event interpretations are written by a separate Lambda, not during primary ingestion.
- Both `soroban_invocations` and `soroban_events` are monthly partitioned by `created_at`.

## Implementation Notes

**File modified:** `libs/domain/src/index.ts` — added 77 lines (10 → 87 lines total).

**Types added (10):**

- `JsonValue` — recursive JSON-safe type replacing `unknown` for JSONB fields
- `ScVal` — type alias for decoded Soroban ScVal (placeholder until task 0013)
- `BigIntString` — type alias for PostgreSQL BIGINT/BIGSERIAL as string
- `ContractType` — union: 'token' | 'dex' | 'lending' | 'nft' | 'other'
- `ContractFunction` — interface with name, parameters[], returnType
- `ContractMetadata` — typed metadata with optional `functions` field
- `SorobanContract` — 7 fields matching DDL (searchVector excluded)
- `EventType` — union: 'contract' | 'system' | 'diagnostic'
- `SorobanInvocation` — 10 fields matching DDL
- `SorobanEvent` — 8 fields matching DDL
- `InterpretationType` — union: 'swap' | 'transfer' | 'mint' | 'burn'
- `EventInterpretation` — 5 fields matching DDL

## Design Decisions

### From Plan

1. **Exclude searchVector from SorobanContract**: DB-only generated TSVECTOR column, as specified in task.

2. **ScVal as JsonValue alias**: Task notes recommend "generic decoded-ScVal representation" pending task 0013.

### Emerged

3. **BigIntString type alias for BIGINT columns**: Plan used `number`, but PostgreSQL BIGINT (2^63-1) exceeds JavaScript safe integer (2^53-1). Drizzle ORM may return BIGINT as string depending on driver config. Using `string` with a semantic alias is safer and self-documenting.

4. **JsonValue instead of unknown for JSONB fields**: `unknown` admits `undefined`, functions, symbols — values impossible in JSON. `JsonValue` constrains to the actual JSON value space.

5. **ContractMetadata interface with typed functions field**: Plan had `Record<string, unknown>` for metadata. Since task defines `ContractFunction` and DDL notes "may include interface signatures", created a dedicated interface linking them.

6. **readonly on array properties**: `parameters`, `topics` use `readonly` arrays — domain types should be immutable by default. Object-level `Readonly<>` used on `structuredData`.

7. **Strict DDL nullability alignment**: Cross-referenced three sources (task 0010 DDL tables, task 0018 DB schema task, master schema doc) to ensure every nullable/non-null field matches. Found and fixed 5 fields that were incorrectly non-null: `metadata`, `contractId` (on invocation and event), `functionArgs`, `returnValue`.

## Issues Encountered

- **No issues encountered.** Straightforward type definition task with no runtime code.
