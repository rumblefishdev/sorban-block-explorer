---
id: '0002'
title: 'Research: LedgerCloseMeta structure and @stellar/stellar-sdk XDR parsing'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0005', '0052', '0053', '0054', '0055']
tags: [priority-high, effort-medium, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
---

# Research: LedgerCloseMeta structure and @stellar/stellar-sdk XDR parsing

## Summary

Investigate the internal structure of LedgerCloseMeta XDR and the concrete `@stellar/stellar-sdk` TypeScript APIs needed to extract all explorer data fields. This research covers both the primary ingestion-time parsing path (Ledger Processor Lambda) and the secondary API-time on-demand decode path (NestJS), and must produce a field-by-field extraction mapping for every database table the pipeline populates.

## Status: Backlog

## Context

The block explorer treats LedgerCloseMeta as its sole canonical chain input. Everything the explorer needs is present in this artifact -- no external API (Horizon, Soroban RPC, third-party indexer) is required for core functionality. XDR parsing is the translation layer between canonical Stellar payloads and the structured PostgreSQL data model.

### Two Parsing Paths

There are two places where XDR parsing happens in the system:

1. **Ingestion-time parsing (primary)** -- The Ledger Processor Lambda fully deserializes every ledger's LedgerCloseMeta using `@stellar/stellar-sdk` XDR types. This is the default path that writes structured explorer records once, avoiding repeated reparsing.

2. **API-time parsing (secondary)** -- The NestJS API retains a narrow, on-demand decode role for advanced transaction views and validation/debug paths that need fields not part of the standard stored read model. This path should remain narrow and must not become the primary materialization layer.

### LedgerHeader Fields

From LedgerHeader, the parser extracts: `sequence`, `closeTime`, `protocolVersion`, `baseFee`, and `txSetResultHash`. These fields anchor ledger ordering, freshness checks, and high-level network statistics.

### TransactionEnvelope and TransactionResult Fields

From TransactionEnvelope and TransactionResult, the parser extracts: `hash` (computed as SHA-256 of the envelope XDR bytes), `sourceAccount`, `feeCharged`, `successful`, and `resultCode`. In addition, the following raw payloads are retained verbatim for advanced inspection: `envelopeXdr`, `resultXdr`, `resultMetaXdr`. These raw artifacts support the advanced transaction view and transaction-tree debugging.

### Operation-Level Data

From OperationMeta per transaction, the parser extracts operation `type` and structured `details` stored as JSONB with type-specific decoded fields. For `INVOKE_HOST_FUNCTION` operations specifically, the parser additionally extracts: `contractId`, `functionName`, `functionArgs` (decoded from ScVal), and `returnValue` (decoded from ScVal).

### Soroban Event Data

From `SorobanTransactionMeta.events`, the parser extracts: `eventType`, `contractId`, `topics` (decoded from ScVal[]), and `data` (decoded from ScVal). These are CAP-67 contract events stored in the `soroban_events` table as structured JSONB. Known NFT-related event patterns may also be interpreted into derived NFT ownership and metadata updates.

### LedgerEntryChanges

From LedgerEntryChanges, the parser extracts derived state for explorer entities:

- **Contract deployments**: `contractId`, `wasmHash`, `deployerAccount` -- used to populate the `soroban_contracts` table
- **Account state snapshots**: `sequence_number`, `balances`, `home_domain` -- used to populate the `accounts` table
- **Liquidity pool state**: `poolId`, asset pair, reserves, total shares -- used to populate `liquidity_pools` and `liquidity_pool_snapshots` tables

### Invocation Tree

Complex Soroban transactions may contain nested contract-to-contract calls. The parser must decode the full invocation hierarchy from `result_meta_xdr` and store it in `transactions.operation_tree` as JSONB. The raw `result_meta_xdr` is preserved alongside the decoded tree for advanced decode/debug use.

### ScVal Decoded Types

The return value of `invokeHostFunction` and event topics/data are XDR ScVal values. These must be decoded into typed representations: integer, string, address, bytes, map, and list. The decoded values are stored with `soroban_invocations` and `soroban_events` so contract history can be served without request-time decode.

### Error Handling

- **Malformed XDR**: If `fromXDR()` throws during ingestion, the Ledger Processor logs the error with transaction context, stores raw XDR verbatim, marks the transaction record with `parse_error`, and keeps it visible with all non-XDR fields still available.
- **Unknown operation types**: New protocol versions may introduce unsupported operation types. These are rendered as "unknown" in explorer responses, raw XDR shown in advanced view, and an operational alarm raised so SDK support can be updated.
- **Protocol upgrades**: Handled by updating `@stellar/stellar-sdk` XDR types. Protocol upgrades are infrequent and announced in advance.

## Research Questions

- Which specific `@stellar/stellar-sdk` TypeScript types and methods are used to deserialize LedgerCloseMeta from zstd-compressed XDR bytes?
- What is the exact method for computing the transaction hash (SHA-256 of envelope XDR bytes) -- is there an SDK helper, or must it be done manually?
- What SDK API is used for ScVal decoding, and what are the TypeScript type signatures for each ScVal variant (integer, string, address, bytes, map, list)?
- How is the invocation tree hierarchy extracted from `result_meta_xdr`? What SDK types represent nested contract-to-contract calls?
- How are LedgerEntryChanges iterated and classified by entry type (contract, account, liquidity pool)?
- What is the difference between `SorobanTransactionMeta.events` event types: 'contract', 'system', 'diagnostic'?
- How are operation types enumerated in the SDK, and what type-specific fields exist for each classic operation vs INVOKE_HOST_FUNCTION?
- What is the SDK version compatibility story -- does a given SDK version support all protocol versions, or only specific ranges?
- What is the performance profile of full LedgerCloseMeta deserialization -- are there known bottlenecks for Lambda execution?

## Acceptance Criteria

- [ ] Complete field-by-field extraction mapping: LedgerCloseMeta field -> database column, for every table in the schema
- [ ] SDK type and method reference for each extraction step (LedgerHeader, TransactionEnvelope, TransactionResult, OperationMeta, SorobanTransactionMeta, LedgerEntryChanges)
- [ ] Transaction hash computation method documented with code example
- [ ] ScVal decode API documented with TypeScript type signatures for all variant types
- [ ] Invocation tree extraction method documented with hierarchy representation
- [ ] Error handling strategy validated: malformed XDR, unknown ops, protocol upgrades
- [ ] Performance estimate for full LedgerCloseMeta parse in Lambda context
- [ ] Raw payload retention strategy confirmed (envelopeXdr, resultXdr, resultMetaXdr storage format)

## Notes

- The database schema has approximately 10 JSONB columns across all tables. Understanding which ScVal shapes map to which JSONB structures is critical for consistent storage.
- GIN indexes exist on `operations.details` and `soroban_events.topics` JSONB columns, so the structure of decoded JSONB must be query-friendly.
- The `transactions.operation_tree` JSONB column stores the full invocation hierarchy -- its shape must support the frontend tree renderer.
