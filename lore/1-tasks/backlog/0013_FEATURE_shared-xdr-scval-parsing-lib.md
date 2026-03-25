---
id: '0013'
title: 'Shared XDR/ScVal parsing utilities library'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0002', '0052', '0053', '0054', '0055', '0027']
tags: [priority-high, effort-medium, layer-domain]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Shared XDR/ScVal parsing utilities library

## Summary

Implement a shared XDR and ScVal parsing utilities library at `libs/shared/src/xdr/`. This library provides the core decode and extraction functions used by both `apps/indexer` (ingestion-time parsing) and `apps/api` (on-demand advanced view decode). It wraps `@stellar/stellar-sdk` XDR types and produces the structured JSONB payloads stored in PostgreSQL.

## Status: Backlog

**Current state:** Not started. Foundation for all ingestion and API decode paths.

## Context

The block explorer's parsing strategy has two paths: ingestion-time (primary, full decode) and API-time (secondary, narrow on-demand decode for advanced views). Both paths need the same underlying decode functions. Centralizing these in `libs/shared/src/xdr/` prevents duplication and ensures consistent output shapes.

All parsing uses `@stellar/stellar-sdk` XDR types. The library does not call Horizon, Soroban RPC, or any external API.

### ScVal-to-JSON decoder

Handle the following ScVal discriminated types, producing tagged JSON:

- **Integer types**: i32, i64, i128, i256, u32, u64, u128, u256
- **String**: string, symbol
- **Address**: account address, contract address
- **Bytes**: raw bytes (hex-encoded)
- **Map**: key-value pairs (recursive decode)
- **List/Vec**: ordered elements (recursive decode)

Output uses discriminated union or tagged JSON so consumers can distinguish types without ambiguity.

### LedgerEntryChanges extractors

Extract derived explorer state from `LedgerEntryChanges`:

- **Contract deployment**: contractId, wasmHash, deployerAccount
- **Account state**: accountId, sequenceNumber, balances, homeDomain
- **Liquidity pool state**: poolId, assetPair, reserves, totalShares

These extractors feed the derived entity tables (soroban_contracts, accounts, liquidity_pools).

### Invocation tree decoder

Decode nested contract-to-contract call hierarchy from `result_meta_xdr`. Produces the `operation_tree` JSONB structure stored in `transactions.operation_tree`. Handles arbitrary nesting depth for cross-contract invocations.

### Contract interface extractor

Extract public function signatures from contract WASM:

- Function name
- Parameter names and types
- Return type

Output matches the `ContractFunction` domain type. Stored in `soroban_contracts.metadata`.

### Transaction hash computation

Compute SHA-256 of `TransactionEnvelope` XDR bytes, producing a 64-character hex string. This is the canonical transaction hash used as the primary lookup key.

### Memo extraction

Extract memo type and memo value from `TransactionEnvelope`. Supports all Stellar memo types (none, text, id, hash, return).

### Event topic decoder

Decode `ScVal[]` topics array from Soroban events. Each topic is decoded using the ScVal-to-JSON decoder. Output is the JSONB array stored in `soroban_events.topics`.

## Implementation Plan

### Step 1: Set up library structure

Create `libs/shared/src/xdr/` directory with barrel exports. Add `@stellar/stellar-sdk` as a dependency.

### Step 2: Implement ScVal-to-JSON decoder

Build the core `decodeScVal(scVal: xdr.ScVal): DecodedScVal` function handling all listed types with tagged JSON output and recursive map/list support.

### Step 3: Implement transaction hash computation

Build `computeTransactionHash(envelopeXdr: string, networkPassphrase: string): string` using SHA-256 over envelope bytes.

### Step 4: Implement memo extraction

Build `extractMemo(envelope: xdr.TransactionEnvelope): { memoType: string | null; memo: string | null }`.

### Step 5: Implement event topic decoder

Build `decodeEventTopics(topics: xdr.ScVal[]): DecodedScVal[]` using the ScVal decoder.

### Step 6: Implement LedgerEntryChanges extractors

Build extractors for contract deployments, account state, and liquidity pool state from `LedgerEntryChanges`.

### Step 7: Implement invocation tree decoder

Build `decodeInvocationTree(resultMetaXdr: string): OperationTree` that recursively decodes the nested contract call hierarchy.

### Step 8: Implement contract interface extractor

Build `extractContractInterface(wasmBytes: Buffer): ContractFunction[]` that parses WASM to extract public function signatures.

### Step 9: Export and test

Export all functions from the barrel file. Write unit tests for each decoder with representative XDR inputs.

## Acceptance Criteria

- [ ] Library located at `libs/shared/src/xdr/` with barrel exports
- [ ] ScVal-to-JSON decoder handles integer, string, address, bytes, map, list types
- [ ] ScVal output uses discriminated union or tagged JSON format
- [ ] LedgerEntryChanges extractors produce contract deployment, account state, and pool state records
- [ ] Invocation tree decoder produces nested hierarchy from result_meta_xdr
- [ ] Contract interface extractor produces ContractFunction[] from WASM
- [ ] Transaction hash computation produces 64-char hex from envelope XDR
- [ ] Memo extraction handles all Stellar memo types
- [ ] Event topic decoder produces decoded ScVal array
- [ ] All functions shared between `apps/indexer` and `apps/api`
- [ ] Uses `@stellar/stellar-sdk` XDR types exclusively
- [ ] Unit tests cover each decoder with representative inputs
- [ ] Types compile without errors

## Notes

- This library is the foundation for all XDR-related work across the explorer.
- The ScVal decoder output format must be stable because it is stored as JSONB in multiple tables (soroban_invocations, soroban_events).
- Error handling in decoders should follow the "log, store raw, mark parse_error" principle (see task 0014).
- The invocation tree decoder is critical for the transaction detail page's call tree visualization.
- Contract interface extraction from WASM may fail for some contracts; this is handled gracefully (see task 0014 for ContractMetadataError).
