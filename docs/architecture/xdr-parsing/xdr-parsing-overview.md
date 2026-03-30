# Stellar Block Explorer - XDR Parsing Overview

> This document expands the XDR parsing portion of
> [`technical-design-general-overview.md`](../technical-design-general-overview.md).
> It preserves the same parsing scope and decode/storage assumptions, but specifies the
> model in more detail so it can later serve as input for implementation task planning.

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Architectural Role](#2-architectural-role)
3. [Parsing Strategy](#3-parsing-strategy)
4. [Data Extracted from XDR](#4-data-extracted-from-xdr)
5. [Soroban-Specific Handling](#5-soroban-specific-handling)
6. [Storage Contract](#6-storage-contract)
7. [Error Handling and Compatibility](#7-error-handling-and-compatibility)
8. [Boundaries and Delivery Notes](#8-boundaries-and-delivery-notes)

---

## 1. Purpose and Scope

XDR parsing is the translation layer between canonical Stellar ledger payloads and the
structured explorer data model stored in PostgreSQL and served by the backend API.

This document covers the target design of XDR parsing only. It does not redefine frontend
behavior, backend transport contracts, or the full database schema except where those are
needed to explain decode responsibilities and storage outcomes.

This document describes the intended production parsing model. It is not a description of
current implementation state in the repository, which is still skeletal.

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the
main overview document takes precedence. This file is an XDR-parsing-focused refinement of
that source, not an independent redesign.

## 2. Architectural Role

The block explorer relies on canonical `LedgerCloseMeta` XDR as its only required chain
input. XDR parsing exists to turn that low-level payload into explorer-friendly read models
without relying on Horizon, Soroban RPC, or any third-party explorer API.

The parsing layer has four jobs:

- decode canonical Stellar payloads into structured records at ingestion time
- preserve raw payloads where advanced inspection and debugging still need them
- extract Soroban-specific structures such as invocation trees, events, and contract
  metadata
- keep frontend and normal API responses free from protocol-level decode work

The parsing layer is not a generic XDR inspection service for arbitrary clients. Its main
purpose is to feed the explorer's own storage and read paths.

## 3. Parsing Strategy

### 3.1 Single Parsing Path — Rust at Ingestion Time

> Per ADR 0004: Rust-only XDR parsing — all decode happens at ingestion time. No TS on-demand decode in the API.

All XDR parsing happens exclusively in the Rust Ledger Processor Lambda at ingestion time.
The NestJS API is pure CRUD — it reads pre-materialized data from PostgreSQL.

Every ledger's `LedgerCloseMeta` is fully deserialized in the Ledger Processor Lambda using
`stellar-xdr` (Rust).

This is the sole parsing path because it lets the system:

- write structured explorer records once instead of reparsing the same payload repeatedly
- store normalized transaction, operation, invocation, and event data in PostgreSQL
- keep normal frontend views independent from raw XDR decoding
- centralize protocol-specific interpretation in one pipeline stage
- maintain a single parser in a single language — no dual-language sync on protocol upgrades

Normal explorer behavior should assume that required structured data has already been
materialized during ingestion.

### 3.2 Raw XDR Passthrough for Advanced Views

Raw XDR payloads (`envelope_xdr`, `result_xdr`, `result_meta_xdr`) are stored verbatim in
the database. The API returns them as opaque base64 strings for the advanced transaction
view — no server-side decode. Client-side decode is the user's responsibility.

If a field is missing from the materialized read model, the Rust parser is updated and data
is re-ingested from the stored raw XDR. Nothing is lost.

### 3.4 Frontend Parsing Boundary

The frontend is not expected to parse XDR for normal explorer operation.

The frontend receives pre-decoded data for:

- transaction summaries
- operation lists and details
- Soroban invocations
- Soroban events
- account, token, NFT, and pool views derived from indexed chain state

Raw XDR is exposed only for advanced transaction inspection.

## 4. Data Extracted from XDR

### 4.1 Ledger Header

From `LedgerHeader`, the parsing layer extracts:

- `sequence`
- `closeTime`
- `protocolVersion`
- `baseFee`
- `txSetResultHash`

These fields anchor ledger ordering, freshness checks, and high-level network statistics.

### 4.2 Transaction Envelope and Result

From `TransactionEnvelope` and `TransactionResult`, the parsing layer extracts:

- `hash`, computed by hashing the envelope XDR
- `sourceAccount`
- `feeCharged`
- `successful`
- `resultCode`

In addition to structured fields, the following raw payloads are retained verbatim:

- `envelopeXdr`
- `resultXdr`
- `resultMetaXdr`

These raw artifacts support the advanced transaction view and transaction-tree debugging.

### 4.3 Operation-Level Data

From `OperationMeta` per transaction, the parsing layer extracts:

- operation `type`
- structured `details` JSONB with type-specific decoded fields

For `INVOKE_HOST_FUNCTION`, the parser additionally extracts:

- `contractId`
- `functionName`
- `functionArgs`, decoded from `ScVal`
- `returnValue`, decoded from `ScVal`

The goal is to persist operation-specific structure once, rather than reconstructing it in
API handlers on every request.

### 4.4 Soroban Event Data

From `SorobanTransactionMeta.events`, the parser extracts:

- `eventType`
- `contractId`
- `topics`, decoded from `ScVal[]`
- `data`, decoded from `ScVal`

Known NFT-related event patterns may also be interpreted into derived NFT ownership and
metadata updates used by explorer-facing NFT views.

### 4.5 Ledger Entry Changes

From `LedgerEntryChanges`, the parser extracts derived state used by explorer entities.

Current documented outputs include:

- contract deployments: `contractId`, `wasmHash`, `deployerAccount`
- account state snapshots: `sequence_number`, `balances`, `home_domain`
- liquidity pool state changes: `poolId`, asset pair, reserves, total shares

This stage is where low-level ledger changes are translated into query-oriented explorer
records.

## 5. Soroban-Specific Handling

### 5.1 CAP-67 Events

CAP-67 contract events are decoded at ingestion time and stored in the
`soroban_events` table as structured JSONB.

This means:

- the backend serves decoded event data directly
- the frontend does not need raw event XDR for normal event rendering
- event interpretation jobs can work from structured persisted events rather than reparsing
  chain payloads repeatedly

### 5.2 Return Values

The return value of `invokeHostFunction` is an XDR `ScVal` and is decoded into a typed
representation such as integer, string, address, bytes, map, or list.

The decoded value is stored with `soroban_invocations` so contract invocation history can be
served without request-time decode for the common case.

### 5.3 Invocation Tree

Complex Soroban transactions may contain nested contract-to-contract calls.

The source design requires the parser to:

- decode the full invocation hierarchy from `result_meta_xdr`
- store that hierarchy in `transactions.operation_tree`
- preserve the raw `result_meta_xdr` alongside the decoded tree for advanced decode/debug use

This lets the transaction detail page render the call tree directly while still keeping the
underlying raw protocol artifact for validation or troubleshooting.

### 5.4 Contract Interface Extraction

Public function signatures are extracted from contract WASM at deployment time.

The extracted interface data is stored inside `soroban_contracts.metadata` and is used by
contract-detail and contract-interface responses.

This extraction is part of the broader XDR/protocol decode pipeline because it turns deployment-related protocol artifacts into stable explorer-facing contract metadata.

## 6. Storage Contract

### 6.1 Raw and Structured Forms Are Both Deliberate

The design intentionally stores both raw and derived representations where needed.

Raw artifacts retained for advanced inspection:

- `transactions.envelope_xdr`
- `transactions.result_xdr`
- `transactions.result_meta_xdr`

Structured artifacts retained for normal explorer reads:

- `operations.details`
- `transactions.operation_tree`
- `soroban_invocations`
- `soroban_events`
- explorer-facing derived entities such as accounts, tokens, NFTs, and liquidity pools

This is not accidental duplication. It is a deliberate tradeoff to support both fast
explorer reads and protocol-level debugging.

### 6.2 Ingestion Owns Materialization

The ingestion path owns writing structured decode results into PostgreSQL.

That includes:

- ledger and transaction summaries
- operation records
- Soroban invocation rows
- Soroban event rows
- derived account, token, NFT, and liquidity-pool state where current product scope needs it

The API reads those materialized results. It should not become the main materialization
layer.

### 6.3 Advanced View Contract

The advanced transaction experience depends on raw payload retention.

The source design currently assumes:

- `envelope_xdr` and `result_xdr` are returned to the frontend as opaque base64 strings for advanced inspection
- `result_meta_xdr` remains stored for potential re-ingestion if Rust parser is updated
- the API does not decode raw XDR server-side — raw payloads are passthrough only (per ADR 0004)

That contract should remain stable unless the main design document is updated first.

## 7. Error Handling and Compatibility

### 7.1 Malformed XDR

If `fromXDR()` throws during ingestion:

- the Ledger Processor logs the error with transaction context
- raw XDR is still stored verbatim
- the transaction record is marked with `parse_error`
- the transaction remains visible with all non-XDR fields that are still available

This preserves explorer continuity even when a decode step fails.

### 7.2 Unknown Operation Types

New protocol versions may introduce operation types not yet supported by the SDK.

In that case, the documented behavior is:

- render the operation as unknown in explorer responses
- show raw XDR in the advanced view
- raise operational visibility through logging/alarming so SDK support can be updated

### 7.3 Protocol Upgrades

When Stellar introduces protocol changes that affect `LedgerCloseMeta` structure, the
system updates the Rust `stellar-xdr` crate in the Ledger Processor (per ADR 0004).

The parsing design assumes protocol upgrades are:

- infrequent
- announced in advance
- handled by updating the decode layer rather than redesigning the explorer architecture

## 8. Boundaries and Delivery Notes

### 8.1 Boundary with Other Parts of the System

Responsibility should remain split clearly:

- ingestion (Rust) owns canonical decode and materialization — single parser, single language
- the database schema owns persistence of raw and structured decode outputs
- the backend (NestJS) owns request-time normalization and raw XDR passthrough — no server-side decode (per ADR 0004)
- the frontend consumes pre-materialized data and does not own normal XDR parsing

### 8.2 Current Workspace State

The repository currently documents the target parsing model but does not yet contain the
final runtime implementation of the Ledger Processor or API-side decode helpers.

That is expected. This document should serve as the detailed reference for future parsing
implementation planning, while
[`technical-design-general-overview.md`](../technical-design-general-overview.md) remains
the primary source of truth.
