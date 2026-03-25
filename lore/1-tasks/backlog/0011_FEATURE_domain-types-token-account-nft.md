---
id: '0011'
title: 'Domain types: token, account, NFT models'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0011', '0012']
tags: [priority-high, effort-small, layer-domain]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Domain types: token, account, NFT models

## Summary

Define the shared TypeScript domain types for tokens, accounts, and NFTs. These types live in `libs/domain` and are consumed by both `apps/api` and `apps/indexer`. They mirror the PostgreSQL schema for the derived explorer entities and the API response contracts.

## Status: Backlog

**Current state:** Not started. Depends on DB schema tasks 0011 and 0012.

## Context

Tokens, accounts, and NFTs are derived, query-oriented explorer entities built on indexed chain state. They unify classic Stellar assets and Soroban-native constructs into a single explorer-facing model. Shared types prevent drift between the ingestion pipeline (which writes these records) and the API (which reads them).

### Token fields (from DDL)

| Field         | DB Type                 | Notes                                         |
| ------------- | ----------------------- | --------------------------------------------- |
| id            | SERIAL PRIMARY KEY      |                                               |
| assetType     | VARCHAR(10) NOT NULL    | CHECK constraint: 'classic', 'sac', 'soroban' |
| assetCode     | VARCHAR(12) nullable    |                                               |
| issuerAddress | VARCHAR(56) nullable    |                                               |
| contractId    | VARCHAR(56) FK nullable | References soroban_contracts(contract_id)     |
| name          | VARCHAR(100) nullable   |                                               |
| totalSupply   | NUMERIC(28,7) nullable  |                                               |
| holderCount   | INT DEFAULT 0           |                                               |
| metadata      | JSONB nullable          |                                               |

Constraints: `UNIQUE(assetCode, issuerAddress)`, `UNIQUE(contractId)`.

Classic tokens are identified by `assetCode + issuerAddress`. Soroban tokens are identified by `contractId`.

### AssetType enum

```typescript
'classic' | 'sac' | 'soroban';
```

### Account fields (from DDL)

| Field           | DB Type                     | Notes                        |
| --------------- | --------------------------- | ---------------------------- |
| accountId       | VARCHAR(56) PRIMARY KEY     |                              |
| firstSeenLedger | BIGINT FK                   | References ledgers(sequence) |
| lastSeenLedger  | BIGINT FK                   | References ledgers(sequence) |
| sequenceNumber  | BIGINT nullable             |                              |
| balances        | JSONB NOT NULL DEFAULT '[]' |                              |
| homeDomain      | VARCHAR(255) nullable       |                              |

Index: `idx_last_seen (last_seen_ledger DESC)`.

Derived-state entity with ledger-sequence watermarks. Older batches cannot overwrite newer state. Account scope is intentionally limited to summary, balances, and recent transactions.

### NFT fields (from DDL)

| Field          | DB Type               | Notes                                     |
| -------------- | --------------------- | ----------------------------------------- |
| id             | BIGSERIAL PRIMARY KEY |                                           |
| contractId     | VARCHAR(56) FK        | References soroban_contracts(contract_id) |
| tokenId        | VARCHAR(128) NOT NULL |                                           |
| collectionName | VARCHAR(100) nullable |                                           |
| ownerAccount   | VARCHAR(56) nullable  |                                           |
| name           | VARCHAR(100) nullable |                                           |
| mediaUrl       | TEXT nullable         |                                           |
| metadata       | JSONB nullable        |                                           |
| mintedAtLedger | BIGINT FK nullable    | References ledgers(sequence)              |
| lastSeenLedger | BIGINT FK nullable    | References ledgers(sequence)              |

Constraint: `UNIQUE(contractId, tokenId)`.

Transfer history is derived from stored events, not a separate table. Metadata and mediaUrl remain optional because NFT contract conventions vary heavily.

## Implementation Plan

### Step 1: Define AssetType enum

Create the `AssetType` union type: 'classic' | 'sac' | 'soroban'.

### Step 2: Define Token domain type

Create `Token` type with all DDL fields. Document that classic tokens use assetCode+issuerAddress identity while Soroban tokens use contractId.

### Step 3: Define Account domain type

Create `Account` type with all DDL fields. Note the ledger-sequence watermark pattern (firstSeenLedger, lastSeenLedger) and that balances is a JSONB array.

### Step 4: Define NFT domain type

Create `NFT` type with all DDL fields. Note that tokenId uniqueness is scoped by contractId and that many fields are nullable due to variable NFT contract conventions.

### Step 5: Export and verify

Export all types from `libs/domain` barrel file. Verify compilation and field alignment with DDL.

## Acceptance Criteria

- [ ] `AssetType` union type defined: 'classic' | 'sac' | 'soroban'
- [ ] `Token` type defined with all DDL fields, both UNIQUE constraints documented
- [ ] `Account` type defined with all DDL fields, balances as JSONB array, watermark pattern noted
- [ ] `NFT` type defined with all DDL fields, UNIQUE(contractId, tokenId) documented
- [ ] All types exported from `libs/domain` barrel
- [ ] Types compile without errors
- [ ] Field names, nullability, and types match the DDL

## Notes

- Classic assets: identified by `assetCode + issuerAddress`.
- SAC (Stellar Asset Contract): a classic asset wrapped as a Soroban contract, has both classic and contract identities.
- Soroban tokens: identified by `contractId` only.
- Account balances are stored as JSONB and updated via ledger-sequence watermarks to prevent stale overwrites.
- NFT transfer history comes from stored Soroban events, not a dedicated transfers table.
- NFT metadata and media fields are nullable because contract conventions are not standardized.
