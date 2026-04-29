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

This document covers the current XDR parsing design. It does not redefine frontend
behavior, backend transport contracts, or the full database schema except where those are
needed to explain decode responsibilities and storage outcomes.

The parsing implementation lives in `crates/xdr-parser/` (shared between the ingest
Lambda and the backend API).

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the
main overview document takes precedence. This file is an XDR-parsing-focused refinement of
that source, not an independent redesign — kept in sync with the code per
[ADR 0032](../../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md).

## 2. Architectural Role

The block explorer relies on canonical `LedgerCloseMeta` XDR as its only required chain
input. XDR parsing exists to turn that low-level payload into explorer-friendly read models
without relying on Horizon, Soroban RPC, or any third-party explorer API.

The parsing layer has four jobs:

- decode canonical Stellar payloads into typed summary records + appearance indexes
  at ingestion time (ingest path)
- re-decode heavy-field XDR at request time for the two detail endpoints that need
  it (E3, E14 per
  [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)),
  fetching `.xdr.zst` from the public Stellar ledger archive
- extract Soroban-specific structures such as invocation trees, events, and contract
  metadata
- keep frontend and normal API responses free from protocol-level decode work

The parser itself is shared: both the ingest Lambda and the backend API link
`crates/xdr-parser`. The parsing layer is not a generic XDR inspection service for
arbitrary clients. Its main purpose is to feed the explorer's own storage and read
paths.

## 3. Parsing Strategy

### 3.1 Two Parsing Paths, One Rust Parser

> Per [ADR 0004](../../../lore/2-adrs/0004_rust-only-xdr-parsing.md): Rust-only XDR
> parsing — the shared `crates/xdr-parser` crate is the single decoder.
> Per [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md):
> raw XDR is not stored in RDS; heavy-field endpoints re-parse from the public
> Stellar ledger archive at read time.

**Ingest path (Ledger Processor Lambda).** Every ledger's `LedgerCloseMeta` is
fully deserialized with `stellar-xdr` via `crates/xdr-parser`. The ingest extracts
typed summary columns + appearance-index rows (per
[ADR 0027](../../../lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md) /
[ADR 0033](../../../lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md) /
[ADR 0034](../../../lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md))
and commits them in a single atomic per-ledger DB transaction via the 14-step
`persist_ledger` method.

**Read path (axum API).** For two endpoints the API fetches the relevant
`.xdr.zst` from the public archive on demand, decompresses it with zstd, and
re-parses with the same shared `crates/xdr-parser`:

- **E3 `/transactions/:hash`** — fetches envelope + result-meta to expand the
  operation list into a full invocation tree, render decoded events, and carry
  `envelope_xdr` / `result_xdr` / `result_meta_xdr` in the advanced view
- **E14 `/contracts/:id/events`** — fetches result-meta for every appearance
  row returned by `soroban_events_appearances` to produce decoded event detail

List endpoints never invoke the parser at read time — they answer from typed
summary columns and appearance indexes.

Using one parser crate for both paths means no dual-language sync on protocol
upgrades and no decode drift between ingest and read.

### 3.2 What Is Not Stored

Per ADR 0029 the following are **not** stored in RDS:

- `envelope_xdr`, `result_xdr`, `result_meta_xdr` as strings or blobs on the
  `transactions` row
- decoded invocation-tree JSONB (`transactions.operation_tree` does not exist)
- full decoded event payload (no `soroban_events` JSONB table — only the
  `soroban_events_appearances` index)
- per-node invocation detail (no `soroban_invocations` row per node — only
  the `soroban_invocations_appearances` index)

All of these are re-derived at request time from the public archive by the
read-path code in §3.1.

### 3.4 Frontend Parsing Boundary

The frontend is not expected to parse XDR for normal explorer operation.

The frontend receives pre-decoded data for:

- transaction summaries
- operation lists and details
- Soroban invocations
- Soroban events
- account, asset, NFT, and pool views derived from indexed chain state

Raw XDR is exposed only for advanced transaction inspection.

## 4. Data Extracted from XDR

### 4.1 Ledger Header

From the enclosing `LedgerHeaderHistoryEntry` and its `LedgerHeader`, the
parsing layer extracts:

- `hash` — the canonical Stellar ledger hash, taken **directly** from
  `LedgerHeaderHistoryEntry.hash` (already populated by stellar-core).
  Never recomputed by the parser — that is the value Horizon
  (`/ledgers/:N.hash`) and every other Stellar tool publishes
- `sequence`
- `closeTime`
- `protocolVersion`
- `baseFee`
- `txSetResultHash`

These fields anchor ledger ordering, freshness checks, and high-level network statistics.

### 4.2 Transaction Envelope and Result

From `TransactionEnvelope` and `TransactionResult`, the ingest path extracts typed
summary columns:

- `hash`, stored as `BYTEA(32)`
  ([ADR 0024](../../../lore/2-adrs/0024_hashes-bytea-binary-storage.md))
- `source_id`, resolved from the source StrKey to `accounts.id`
  ([ADR 0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md))
- `fee_charged`, `successful`
- `application_order`, `operation_count`, `has_soroban`, `inner_tx_hash` (fee-bump)
- `result_code` is not persisted at ingest; it is re-derived on demand from
  the archive for the advanced view

Raw envelope / result / result-meta XDR is **not** retained in RDS (ADR 0029).
The advanced transaction view pulls the corresponding `.xdr.zst` from the public
archive at request time.

### 4.3 Operation-Level Data (Appearance Index)

Per task 0163, `operations` was collapsed to an appearance index and renamed
to `operations_appearances`. Ingest aggregates operations by identity at
staging time (`HashMap<OpIdentity, i64>`), writing one row per distinct
identity per transaction with `amount BIGINT` counting collapsed duplicates.

From `OperationMeta` per transaction, the ingest path extracts:

- operation `type` as `SMALLINT` backed by the Rust `OperationType` enum
  ([ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md))
- `source_id`, `destination_id` surrogate FKs (ADR 0026)
- `contract_id` surrogate FK
  ([ADR 0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md))
- `asset_code`, `asset_issuer_id`, `pool_id` (BYTEA 32)
- `ledger_sequence`, `created_at`
- `amount` aggregate count of physical operations collapsed into this identity

Not stored at ingest (re-derived from XDR at read time per ADR 0029):
`transfer_amount` (dropped), `application_order` (dropped), per-op JSONB
`details` (never existed), envelope/args/memo/predicates decode.

For `INVOKE_HOST_FUNCTION`, ingest captures only the appearance-index rows
(§4.4 / §4.5). The `functionName`, decoded `functionArgs`, `returnValue`, and
per-node invocation tree are re-expanded at request time from the archive by
the E3 read path.

### 4.4 Soroban Event Data (Ingest: Appearance Index)

From `SorobanTransactionMeta.events`, the ingest path extracts one
**appearance-index row** per `(contract, tx, ledger)` trio in
`soroban_events_appearances` with:

- `contract_id` surrogate FK (ADR 0030), `transaction_id`, `ledger_sequence`,
  `created_at`
- `amount` = count of non-diagnostic events in the trio

Full decoded event detail (`eventType` as `SMALLINT`, `topics` as decoded
`ScVal[]`, `data` as decoded `ScVal`) is **not** stored. Known NFT-related
event patterns are still interpreted at ingest into derived state updates on
`nfts` / `nft_ownership` / `assets` (classification happens by looking at the
events without persisting them).

At read time, `xdr_parser::extract_events` re-expands the decoded payload from
the archive for E14 `/contracts/:id/events`.

### 4.5 Soroban Invocations (Ingest: Appearance Index)

Mirroring §4.4, ingest writes one row per `(contract, tx, ledger)` trio to
`soroban_invocations_appearances` with:

- surrogate `contract_id`, `transaction_id`, `ledger_sequence`, `created_at`
- `caller_id` — the root-level caller `accounts.id`, NULL for C-contract
  sub-invocation callers
- `amount` = count of invocation-tree nodes in the trio

Per-node decode (function name, args, return value, depth) happens at read
time in `xdr_parser::extract_invocations` for E3 and E11 /
E-contract-invocations endpoints.

### 4.6 Ledger Entry Changes

From `LedgerEntryChanges`, the parser extracts derived state used by explorer
entities:

- contract deployments → `soroban_contracts` row (contract_id surrogate, wasm_hash
  BYTEA 32, deployer_id surrogate, is_sac, contract_type SMALLINT)
- WASM upload → `wasm_interface_metadata` row (SEP-48-derived JSONB, keyed by
  wasm_hash BYTEA)
- account state → `accounts` row + `account_balances_current` entries per
  trustline / native (balances are typed `NUMERIC(28,7)` per-asset rows, not a
  JSONB blob on `accounts`)
- classic LP state → `liquidity_pools` row + `liquidity_pool_snapshots` row +
  `lp_positions` upsert per participating account (asset pair modeled as typed
  `asset_*_type SMALLINT` + code + issuer_id, not JSONB)

This stage is where low-level ledger changes are translated into query-oriented
explorer records.

## 5. Soroban-Specific Handling

### 5.1 CAP-67 Events

CAP-67 contract events follow the **appearance-index + read-time decode** pattern
per [ADR 0033](../../../lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md):

- at ingest, one row per `(contract, tx, ledger)` trio is written to
  `soroban_events_appearances` with a non-diagnostic-event count — no decoded
  event type, topics, or data are persisted
- at read time, E14 re-parses the archive via `xdr_parser::extract_events` and
  renders decoded `ScVal` topics / data per event
- known NFT / SEP-41 patterns are still interpreted at ingest to drive
  `assets` / `nfts` / `nft_ownership` upserts, but the triggering events
  themselves are not retained as rows

#### V3 vs V4 meta dispatch (Protocol 22 ↔ Protocol 23+)

`xdr_parser::extract_events` dispatches on the `TransactionMeta` variant
because Protocol 23 (CAP-67) reorganised the on-chain event surface
([ADR 0002](../../../lore/2-adrs/0002_rust-ledger-processor-lambda.md) §1):

- **V3** (`TransactionMetaV3`, Protocol ≤ 22): all Soroban contract events
  are at `soroban_meta.events`; diagnostic events at
  `soroban_meta.diagnostic_events`. The parser reads both.
- **V4** (`TransactionMetaV4`, Protocol ≥ 23): events live in **three**
  locations and the parser reads all three in this order:

  1. `tx_meta.events` (`VecM<TransactionEvent>`) — transaction-level
     events: fee `BeforeAllTxs` charge, `AfterTx` refund, `AfterAllTxs`.
  2. `tx_meta.operations[i].events` (`OperationMetaV2.events: VecM<ContractEvent>`) —
     per-operation events: Soroban contract events emitted during
     `InvokeHostFunction` execution **and** SAC `transfer` / `mint` / `burn`
     events emitted by classic operations under Protocol 23 unification.
  3. `tx_meta.diagnostic_events` (`VecM<DiagnosticEvent>`) — host-level
     diagnostic / trace events.

  `SorobanTransactionMetaV2` (the V4 `soroban_meta`) no longer carries an
  `events` field — that field was removed in CAP-67. `event_index` is
  numbered sequentially across all three sources within a single
  transaction so the V3 contract (monotonic per-tx index) is preserved.

The split matters because per-operation events carry the bulk of
post-Protocol 23 Soroban traffic. Missing them produces a silently
incomplete `soroban_events_appearances` index for every Protocol ≥ 23
ledger — the canonical symptom is a Soroban tx with exactly two events,
both XLM-SAC fee events at the tx-level location, while the contract's
own `transfer` / `mint` / `burn` events (which lived under
`operations[i].events`) are dropped.

### 5.2 Return Values

Return values of `invokeHostFunction` are decoded from XDR `ScVal` into typed
representations (integer, string, address, bytes, map, list) by
`xdr_parser::extract_invocations` at **request time** — not at ingest.
Ingest only records the appearance-index row in
`soroban_invocations_appearances` (ADR 0034).

### 5.3 Invocation Tree

Complex Soroban transactions may contain nested contract-to-contract calls.

Per [ADR 0034](../../../lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md)
the parser's responsibilities are split:

- **ingest**: write an appearance row per trio with `amount` = node count and
  `caller_id` = root-level account caller (C-contract sub-callers collapsed to
  NULL by the `is_strkey_account` filter so that `COUNT(DISTINCT caller_id)`
  answers E11's `unique_callers` stat directly)
- **read**: E3's transaction-detail renderer pulls the `.xdr.zst` from the
  public archive, decodes the full tree with `xdr_parser::extract_invocations`,
  and returns it as the `operation_tree` field of the response

Raw `result_meta_xdr` is not persisted on `transactions` (ADR 0029); the
archive is the authoritative source.

### 5.4 Contract Interface Extraction

Public function signatures are extracted from contract WASM at deployment time
and stored in `wasm_interface_metadata.metadata` (keyed by `wasm_hash BYTEA(32)`),
deduplicated across every contract instance that shares the same WASM.

`soroban_contracts.metadata` additionally carries contract-instance-level
metadata (name, description, etc. — see
[ADR 0022](../../../lore/2-adrs/0022_schema-correction-and-token-metadata-enrichment.md))
and is updated by the async metadata-enrichment worker.

This extraction is part of the broader XDR/protocol decode pipeline because it
turns deployment-related protocol artifacts into stable explorer-facing contract
metadata.

## 6. Storage Contract

### 6.1 Typed Columns and Appearance Indexes, No Raw XDR

Per [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md),
the DB holds only what list endpoints and partition-pruned reads need.

Typed summary columns / structured artifacts retained for normal explorer reads:

- `transactions` — `hash BYTEA`, `source_id BIGINT`, `fee_charged`, `successful`,
  `application_order`, `operation_count`, `has_soroban`, `inner_tx_hash`,
  `parse_error`, `created_at`
- `operations_appearances` — `type SMALLINT`, surrogate FKs, `BYTEA pool_id`, `amount BIGINT` count,
  typed `asset_code`/`asset_issuer_id`
- `soroban_events_appearances` / `soroban_invocations_appearances` — appearance
  indexes only (per §4.4 / §4.5)
- `soroban_contracts`, `wasm_interface_metadata` — with surrogate PK, BYTEA
  wasm_hash, SMALLINT contract_type, JSONB metadata
- derived explorer entities: `accounts`, `assets`, `nfts`, `nft_ownership`,
  `liquidity_pools`, `liquidity_pool_snapshots`, `lp_positions`,
  `account_balances_current` (the previously-planned
  `account_balance_history` was dropped per
  [ADR 0035](../../../lore/2-adrs/0035_drop-account-balance-history.md))

Raw artifacts **not** retained in the DB (fetched at request time from the
public archive):

- envelope / result / result-meta XDR
- decoded event payload (type, topics, data)
- per-node invocation detail (function name, args, return value, depth)
- full invocation tree

### 6.2 Two Phases of Materialization

Ingestion owns writing typed summary + appearance-index rows into PostgreSQL.
That is the only phase that runs unconditionally per ledger close.

The backend read path owns re-materializing heavy fields on demand for E3 / E14
via `xdr_parser::extract_*`; this phase runs only when a request asks for it
and is cacheable at the API Gateway / CloudFront layer.

### 6.3 Advanced View Contract

The advanced transaction experience is served by the read path, not by stored
raw payloads:

- E3 `/transactions/:hash` fetches the relevant `.xdr.zst` from the public
  archive, decompresses and parses it, and returns `envelope_xdr`, `result_xdr`,
  `result_meta_xdr`, `operation_tree`, and decoded events in the response
- response fields preserve their historical names so the public API surface is
  unchanged
- if the Rust parser is updated to expose a new field, no re-ingest is needed —
  the archive is the canonical source; the next request for a given transaction
  just picks up the new field

This contract should remain stable unless the main design document is updated
first.

## 7. Error Handling and Compatibility

### 7.1 Malformed XDR

If `stellar-xdr` returns an error during ingestion:

- the Ledger Processor logs the error with the transaction hash
- the typed summary columns that were successfully extracted are still written
- `transactions.parse_error = true` is set on the affected row
- the transaction remains visible with all non-failed fields available

At read time, E3 retries the archive fetch and re-parse on its own retry budget.
If that also fails, the detail endpoint returns a decode-failure marker in the
response; list endpoints are not affected because they do not call the archive.

### 7.2 Unknown Operation Types

New protocol versions may introduce operation types not yet supported by the
pinned `stellar-xdr` crate.

In that case, the documented behavior is:

- render the operation as unknown in explorer responses
- surface the raw XDR (fetched from the archive) in the advanced view
- raise operational visibility through logging / alarming so the `stellar-xdr`
  bump can be scheduled

### 7.3 Protocol Upgrades

When Stellar introduces protocol changes that affect `LedgerCloseMeta` structure, the
system updates the Rust `stellar-xdr` crate in the Ledger Processor (per ADR 0004).

The parsing design assumes protocol upgrades are:

- infrequent
- announced in advance
- handled by updating the decode layer rather than redesigning the explorer architecture

## 8. Boundaries and Delivery Notes

### 8.1 Boundary with Other Parts of the System

Responsibility is split along the two-path parsing model:

- **ingestion** (Rust Ledger Processor) owns decode-at-ingest → typed summary
  columns + appearance indexes written to PostgreSQL (single parser crate,
  shared with the API — `crates/xdr-parser`)
- **the database schema** owns persistence of typed summaries + appearance
  indexes; it does not hold raw XDR (ADR 0029)
- **the backend** (axum) owns request-time re-decode for E3 / E14 via
  `xdr_parser::extract_*` against `.xdr.zst` fetched from the public Stellar
  ledger archive; list endpoints run no parser
- **the frontend** consumes the API response and does not own XDR parsing in
  normal paths

### 8.2 Current Workspace State

The parsing implementation lives in `crates/xdr-parser/` and is invoked from
both the ingest Lambda (`crates/indexer`) and the backend API (`crates/api`).
Per [ADR 0032](../../../lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md)
this document is kept in sync with the code by requiring ADRs that touch the
parsing path to update it in the same PR.

[`technical-design-general-overview.md`](../technical-design-general-overview.md)
remains the primary cross-component source of truth; this file is the detailed
XDR-parsing reference.
