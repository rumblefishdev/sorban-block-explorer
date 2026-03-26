---
id: '0002'
title: 'Research: LedgerCloseMeta structure and @stellar/stellar-sdk XDR parsing'
type: RESEARCH
status: completed
related_adr: ['0002']
related_tasks: ['0001', '0005', '0052', '0053', '0054', '0055']
tags: [priority-high, effort-medium, layer-research]
links:
  - https://stellar.github.io/js-stellar-sdk/
  - https://github.com/stellar/js-stellar-sdk
  - https://github.com/stellar/js-stellar-base
  - https://developers.stellar.org/docs/networks/software-versions
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
  - date: 2026-03-25
    status: active
    who: stkrolikiewicz
    note: 'Research started - investigating LedgerCloseMeta XDR structure and @stellar/stellar-sdk parsing APIs'
  - date: 2026-03-26
    status: completed
    who: stkrolikiewicz
    note: >
      Research complete. 5 notes (4 R-, 1 S-), 8 archived sources, 8/8 AC met.
      Key findings: Protocol 25 TransactionMetaV4 relocates events, dual-phase TX set,
      invocation tree from auth not meta, ScVal typed JSON format from stellar-indexer.
      Proposed ADR-0002: Rust for Ledger Processor Lambda.
---

# Research: LedgerCloseMeta structure and @stellar/stellar-sdk XDR parsing

## Summary

Investigate the internal structure of LedgerCloseMeta XDR and the concrete APIs needed to extract all explorer data fields. Primary focus on **Rust** (`stellar_xdr::curr` crate) for the Ledger Processor Lambda, with secondary coverage of `@stellar/stellar-sdk` TypeScript for the NestJS API on-demand decode path. Produces a field-by-field extraction mapping for every database table.

## Status: Active

## Research Notes

| Note                                                                                 | Topic                                                                 |
| ------------------------------------------------------------------------------------ | --------------------------------------------------------------------- |
| [R-sdk-types-and-deserialization.md](notes/R-sdk-types-and-deserialization.md)       | Rust `stellar_xdr::curr` types, deserialization, TX hash, performance |
| [R-soroban-events-and-invocations.md](notes/R-soroban-events-and-invocations.md)     | Rust: Events (V3 vs V4), ScVal typed JSON, invocation tree            |
| [R-field-mapping-tables.md](notes/R-field-mapping-tables.md)                         | Rust struct paths → DB column for all 12 tables                       |
| [R-error-handling-and-performance.md](notes/R-error-handling-and-performance.md)     | Rust: Result<T,E> error handling, protocol upgrades, Lambda perf      |
| [S-language-choice-ledger-processor.md](notes/S-language-choice-ledger-processor.md) | Rust vs Go vs TypeScript comparison for Ledger Processor (needs ADR)  |

## Key Findings

- **Rust-first implementation:** Ledger Processor uses `stellar_xdr::curr` crate (v25) — canonical XDR types auto-generated from stellar-core. Exhaustive `match` on enums prevents silent breakage on protocol upgrades.
- **Protocol 25 (TransactionMetaV4):** Events relocated from `soroban_meta.events` to top-level `v4.events`. Rust `match` on `TransactionMeta::V3` vs `V4` is compile-time safe.
- **TX set dual phases:** `TransactionPhase::V0` (classic) + `V1` (parallel Soroban execution). Both iterated in `for_each_envelope()`.
- **TX hash computation:** `env.hash(network_id)` — built-in on `TransactionEnvelope` in Rust. No manual payload construction.
- **ScVal typed JSON:** `{ "type": "u128", "value": "123" }` format from `stellar-indexer` — preserves type info for JSONB/GIN indexes.
- **Performance:** Rust estimated ~5-10ms per heavy ledger (vs 76ms Node.js). ~500x Lambda headroom.
- **Invocation tree:** from `InvokeHostFunctionOp.auth[].root_invocation` with recursive `sub_invocations`.
- **Reference implementation:** `rumblefishdev/stellar-indexer` (Rust) — working V3/V4 event handling, ScVal conversion, envelope extraction. Source files copied to `sources/`.

## Corrections to Architecture Docs

1. **TransactionMeta version:** Architecture docs assume V3 only. Protocol 25 uses V4 which relocates events. Parser must dispatch on `meta.switch()` value (3 or 4).
2. **TX set phases:** Architecture docs describe a single TX set. Protocol 25 has two phases — classic and parallel Soroban execution.
3. **`rumblefishdev/stellar-indexer`:** Internal Rust reference implementation, available locally at `../stellar-indexer`. Key source files copied to `sources/` directory.
4. **Invocation tree source:** Architecture docs say invocation tree comes from `result_meta_xdr`. In reality, it comes from `InvokeHostFunctionOp.auth[].rootInvocation` in the **transaction envelope**, not the meta. The `result_meta_xdr` contains events and LedgerEntryChanges, but not the invocation hierarchy.
5. **Language choice:** Architecture docs assume TypeScript/Node.js for Ledger Processor. Research reveals Rust and Go are also viable — see S-language-choice-ledger-processor.md for comparison. This decision should be formalized as an ADR.

## Research Questions → Answer Location

| #   | Question                                      | Answered In                                                                                                               |
| --- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| 1   | SDK types for LedgerCloseMeta deserialization | [R-sdk-types § LedgerCloseMeta Deserialization](notes/R-sdk-types-and-deserialization.md#ledgerclosemeta-deserialization) |
| 2   | Transaction hash computation                  | [R-sdk-types § Transaction Hash Computation](notes/R-sdk-types-and-deserialization.md#transaction-hash-computation)       |
| 3   | ScVal decode API and type signatures          | [R-soroban-events § ScVal Decode API](notes/R-soroban-events-and-invocations.md#scval-decode-api)                         |
| 4   | Invocation tree hierarchy                     | [R-soroban-events § Invocation Tree](notes/R-soroban-events-and-invocations.md#invocation-tree)                           |
| 5   | LedgerEntryChanges classification             | [R-field-mapping § LedgerEntryChanges Iteration](notes/R-field-mapping-tables.md#ledgerentrychanges-iteration)            |
| 6   | Event types: contract, system, diagnostic     | [R-soroban-events § Event Types](notes/R-soroban-events-and-invocations.md#event-types-contracteventtype)                 |
| 7   | Operation types enumeration                   | [R-sdk-types § Operation Types](notes/R-sdk-types-and-deserialization.md#operation-types-27-total)                        |
| 8   | SDK version compatibility                     | [R-error-handling § Protocol Upgrade Handling](notes/R-error-handling-and-performance.md#protocol-upgrade-handling)       |
| 9   | Performance profile                           | [R-error-handling § Performance Estimates](notes/R-error-handling-and-performance.md#performance-estimates)               |

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

- [x] Complete field-by-field extraction mapping: LedgerCloseMeta field -> database column, for every table in the schema
- [x] SDK type and method reference for each extraction step (LedgerHeader, TransactionEnvelope, TransactionResult, OperationMeta, SorobanTransactionMeta, LedgerEntryChanges)
- [x] Transaction hash computation method documented with code example
- [x] ScVal decode API documented with TypeScript type signatures for all variant types
- [x] Invocation tree extraction method documented with hierarchy representation
- [x] Error handling strategy validated: malformed XDR, unknown ops, protocol upgrades
- [x] Performance estimate for full LedgerCloseMeta parse in Lambda context
- [x] Raw payload retention strategy confirmed (envelopeXdr, resultXdr, resultMetaXdr storage format)

## Design Decisions

### From Plan

1. **TypeScript SDK as primary research target**: Task specified `@stellar/stellar-sdk` — researched as planned.
2. **Field-by-field mapping structure**: One table per DB entity, XDR path + SDK method for each column.

### Emerged

3. **Rust as recommended Ledger Processor language**: Research revealed `stellar-indexer` Rust implementation and stronger XDR type safety. Proposed ADR-0002.
4. **TransactionMetaV4 discovery**: Architecture docs assumed V3 only. Protocol 25 (mainnet) uses V4 with relocated events. Critical for parser correctness.
5. **Typed JSON format for ScVal JSONB**: Adopted `{ "type": "u128", "value": "123" }` pattern from `stellar-indexer` instead of flat `scValToNative()` output. Better for GIN indexes and frontend rendering.
6. **Invocation tree source correction**: Architecture docs said tree comes from `result_meta_xdr`. Actually from `InvokeHostFunctionOp.auth[].rootInvocation` in the envelope.

## Issues Encountered

- **Soroban RPC `getLedgers` returns `LedgerCloseMeta`** (not batch), while Galexie writes `LedgerCloseMetaBatch`. Parser must handle both formats.
- **`meta.switch()` in JS SDK returns raw number** (e.g., `4`) with `undefined` name — required manual version dispatch instead of named enum matching.
- **Protocol 25 dual-phase TX set**: `phase.v0Components()` throws if phase is V1 (parallel). Must use try/catch to detect phase type in JS. Rust uses clean `match`.
- **Public Galexie data lake not accessible**: GCS bucket `sdf-ledger-close-meta` returns 403. Used Soroban RPC `getLedgers` as alternative data source.

## Future Work

- ADR-0002 acceptance (Rust for Ledger Processor) — needs team discussion
- Rust performance benchmarks with `stellar-indexer` against same ledger data
- Classic operation JSONB details per type (payment, changeTrust, etc.) — deferred to implementation tasks 0060-0063

## Notes

- The database schema has approximately 10 JSONB columns across all tables. Understanding which ScVal shapes map to which JSONB structures is critical for consistent storage.
- GIN indexes exist on `operations.details` and `soroban_events.topics` JSONB columns, so the structure of decoded JSONB must be query-friendly.
- The `transactions.operation_tree` JSONB column stores the full invocation hierarchy -- its shape must support the frontend tree renderer.
