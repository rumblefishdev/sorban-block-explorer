---
id: '0010'
title: 'Domain types: Soroban models (contract, invocation, event)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0010']
tags: [priority-high, effort-small, layer-domain]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Domain types: Soroban models (contract, invocation, event)

## Summary

Define the shared TypeScript domain types for Soroban contracts, contract invocations, Soroban events, and event interpretations. These types live in `libs/domain` and are consumed by both `apps/api` and `apps/indexer`. They mirror the PostgreSQL schema for the Soroban-specific tables and the API response contracts.

## Status: Backlog

**Current state:** Not started. Depends on DB schema task for Soroban tables.

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

- [ ] `ContractType` union type defined: 'token' | 'dex' | 'lending' | 'nft' | 'other'
- [ ] `ContractFunction` type defined with name, parameters (name + type), returnType
- [ ] `SorobanContract` type defined with all DDL fields (excluding searchVector)
- [ ] `SorobanInvocation` type defined with all DDL fields, JSONB fields typed appropriately
- [ ] `SorobanEvent` type defined with all DDL fields, eventType union typed
- [ ] `EventInterpretation` type defined with interpretationType union
- [ ] All types exported from `libs/domain` barrel
- [ ] Types compile without errors

## Notes

- `searchVector` is a generated tsvector column in PostgreSQL and should not appear in the domain type.
- `functionArgs` and `returnValue` on invocations are decoded ScVal stored as JSONB; the domain type should use a generic decoded-ScVal representation (see task 0013 for the ScVal parsing library).
- Event interpretations are written by a separate Lambda, not during primary ingestion.
- Both `soroban_invocations` and `soroban_events` are monthly partitioned by `created_at`.
