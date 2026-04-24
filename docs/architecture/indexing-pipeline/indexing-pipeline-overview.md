# Stellar Block Explorer - Indexing Pipeline Overview

> This document expands the indexing pipeline portion of
> [`technical-design-general-overview.md`](../technical-design-general-overview.md).
> It preserves the same ingestion scope and runtime assumptions, but specifies the pipeline
> in more detail so it can later serve as input for implementation task planning.

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Architectural Role](#2-architectural-role)
3. [Pipeline Topology](#3-pipeline-topology)
4. [Canonical Input Model](#4-canonical-input-model)
5. [Live Ingestion Flow](#5-live-ingestion-flow)
6. [Historical Backfill Flow](#6-historical-backfill-flow)
7. [Worker Responsibilities](#7-worker-responsibilities)
8. [Operational Characteristics](#8-operational-characteristics)
9. [Boundaries and Delivery Notes](#9-boundaries-and-delivery-notes)

---

## 1. Purpose and Scope

The indexing pipeline is the system that turns canonical Stellar ledger closes into the
block explorer's own structured PostgreSQL data model.

Its purpose is to ingest chain data once, materialize explorer-facing records, and keep the
API and frontend independent from third-party explorer services or direct chain parsing at
request time.

This document covers the target design of the indexing pipeline only. It does not redefine
frontend behavior, backend transport contracts, or the detailed XDR parsing/storage model
except where those are needed to explain pipeline responsibilities.

This document describes the intended production ingestion model. It is not a description of
current implementation state in the repository, which is still skeletal.

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the main
overview document takes precedence. This file is an indexing-pipeline-focused refinement of
that source, not an independent redesign.

## 2. Architectural Role

The indexing pipeline sits between canonical Stellar data sources and the explorer's owned
PostgreSQL schema.

Its role is to:

- stream or backfill canonical ledger data into the system
- parse `LedgerCloseMeta` payloads into structured explorer records
- persist those records into RDS PostgreSQL
- derive higher-level explorer entities such as contracts, accounts, assets, NFTs, and
  liquidity pools from canonical ledger artifacts
- make all normal backend and frontend reads depend on the explorer's own database rather
  than on external APIs

The pipeline is intentionally not a public API surface. It is an internal ingestion and
materialization boundary.

## 3. Pipeline Topology

### 3.1 End-to-End Flow

The source design defines the indexing pipeline as a fixed event-driven chain:

```text
Stellar Network peers / history archives
  -> Galexie on ECS Fargate
  -> S3 bucket: stellar-ledger-data
  -> Lambda: Ledger Processor
  -> RDS PostgreSQL
```

This same flow is used for both live ingestion and historical backfill.

### 3.2 Main Runtime Components

The pipeline depends on four primary runtime components:

- **Galexie on ECS Fargate** for canonical ledger export
- **S3** for transient `LedgerCloseMeta` object storage
- **Ledger Processor Lambda** for event-driven parsing and persistence
- **RDS PostgreSQL** as the explorer's owned storage target

### 3.3 Why the Pipeline Is Structured This Way

The current design uses S3 as a handoff boundary between ledger export and parse/write work.

That gives the system:

- a durable intermediate artifact per ledger close
- one shared handoff format for live ingestion and backfill
- replayability when downstream processing fails
- clean separation between continuous export and parse/materialization work

## 4. Canonical Input Model

### 4.1 Source of Truth

The indexing pipeline treats `LedgerCloseMeta` as the canonical input artifact.

The source design is explicit that everything the explorer needs is present in
`LedgerCloseMeta`; no external API is required for core explorer functionality.

### 4.2 Data Present in `LedgerCloseMeta`

The current design expects the pipeline to consume at least these categories from the input
artifact:

- ledger sequence, close time, and protocol version from `LedgerHeader`
- transaction hash, source account, fee, and success/failure status from
  `TransactionEnvelope` and `TransactionResult`
- operation type and details from `OperationMeta`
- Soroban invocation data from `InvokeHostFunctionOp` and
  `SorobanTransactionMeta.returnValue`
- CAP-67 contract events from `SorobanTransactionMeta.events`
- contract deployment data from `LedgerEntryChanges` of contract type
- account changes from `LedgerEntryChanges` of account type
- liquidity pool state from `LedgerEntryChanges` of liquidity-pool type

### 4.3 Shared Input Artifact Format

Galexie exports one `LedgerCloseMeta` XDR file per ledger close.

The file format assumptions currently documented are:

- one file per ledger
- zstd-compressed XDR
- written under `stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd`

The pipeline should preserve this artifact contract unless the main overview changes first.

## 5. Live Ingestion Flow

### 5.1 Live Source

Live ingestion uses self-hosted Galexie running continuously on ECS Fargate.

Galexie connects to Stellar network peers through Captive Core and exports ledger-close
artifacts continuously.

The design expectation is roughly one file every 5 to 6 seconds, aligned with ledger-close
cadence.

### 5.2 Live Processing Steps

For each arriving ledger artifact, the current pipeline model is the 14-step
`persist_ledger` method in `crates/indexer/src/handler/persist/mod.rs` per
[ADR 0027](../../../lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md),
committed in a single atomic DB transaction:

1. download and decompress the XDR file from S3
2. parse `LedgerCloseMeta` using the Rust `stellar-xdr` crate (ADR 0004) and
   extract the shared canonical data needed from it via `crates/xdr-parser`;
   persistence-oriented staging/aggregation is then handled in the indexer
3. resolve every observed StrKey (`G...`, `C...`) to the relevant `BIGINT`
   surrogate via `accounts` and `soroban_contracts` (two-pass upsert pattern per
   ADRs [0026](../../../lore/2-adrs/0026_accounts-surrogate-bigint-id.md) /
   [0030](../../../lore/2-adrs/0030_contracts-surrogate-bigint-id.md))
4. write the `ledgers` row (hash as `BYTEA(32)` per
   [ADR 0024](../../../lore/2-adrs/0024_hashes-bytea-binary-storage.md))
5. write `transactions` rows with typed summary columns only — no raw envelope /
   result / result-meta XDR per
   [ADR 0029](../../../lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md);
   hash uniqueness flows through the companion `transaction_hash_index` row
6. aggregate operations by identity (staging-time `HashMap<OpIdentity, i64>`)
   and write `operations_appearances` rows with `type SMALLINT`
   ([ADR 0031](../../../lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md)),
   surrogate FKs, `BYTEA pool_id`, and `amount BIGINT` counting collapsed
   duplicates (per task 0163 — no `transfer_amount`, no `application_order`,
   no JSONB details; pattern from ADRs 0033/0034). Bulk INSERT with
   `ON CONFLICT ON CONSTRAINT uq_ops_app_identity DO NOTHING` for replay
   idempotency
7. write `transaction_participants` appearance rows
8. write `soroban_events_appearances` — one row per `(contract, tx, ledger)`
   with a non-diagnostic-event count (ADR 0033; full detail re-expanded at read
   time via `xdr_parser::extract_events`)
9. write `soroban_invocations_appearances` — one row per trio with invocation-
   node count and the root-level `caller_id` (ADR 0034)
10. upsert `soroban_contracts` and `wasm_interface_metadata` on observed
    deployments + WASM uploads (`wasm_hash` as `BYTEA(32)`, `contract_type`
    as `SMALLINT`)
11. derive and upsert `assets` (native, classic_credit, SAC, soroban) per
    [ADR 0036](../../../lore/2-adrs/0036_rename-tokens-to-assets.md) (table
    renamed from `tokens`; four `asset_type` variants)
12. derive and upsert `nfts`, append `nft_ownership` rows
13. derive and upsert `liquidity_pools`, append `liquidity_pool_snapshots`,
    upsert `lp_positions`
14. upsert `accounts` summary and `account_balances_current`
    (the parallel `account_balance_history` append was removed in task 0159
    per [ADR 0035](../../../lore/2-adrs/0035_drop-account-balance-history.md);
    chart feature design deferred to launch time)

### 5.3 Write Target

The live ingestion path writes directly to the explorer's owned PostgreSQL schema.

That write includes both:

- low-level structured explorer records (`ledgers`, `transactions`, `operations_appearances`,
  `transaction_participants`, and the appearance indexes
  `soroban_events_appearances` / `soroban_invocations_appearances`)
- derived explorer-facing state (`accounts`, `soroban_contracts`,
  `wasm_interface_metadata`, `assets`, `nfts`, `nft_ownership`, `liquidity_pools`,
  `liquidity_pool_snapshots`, `lp_positions`, `account_balances_current`)

List and partition-pruned reads serve entirely from this persisted state.
Heavy-field endpoints (E3 `/transactions/:hash`, E14 `/contracts/:id/events`)
additionally fetch raw `.xdr.zst` from the public Stellar ledger archive and
re-parse at request time per ADR 0029 — that is a **read-path** dependency, not
an ingest-path one; the indexing pipeline itself never calls the public archive.

## 6. Historical Backfill Flow

### 6.1 Backfill Source and Runtime

Per [ADR 0010](../../../lore/2-adrs/0010_local-backfill-over-fargate.md),
historical backfill runs as a **local CLI tool** (`crates/backfill-runner` for
production runs, `crates/backfill-bench` for benchmarks/prototypes)
on a developer workstation. It streams from Stellar's **public history
archives** (the same archives Horizon used for `db reingest`) and writes
directly to the target RDS.

### 6.2 Shared Pipeline, Not Shared Storage

Backfill and live ingestion share the `process_ledger` **code path** (both
run the same 14-step `persist_ledger` via `crates/indexer`), but not the
delivery medium:

- **live:** Galexie (ECS Fargate) → S3 `stellar-ledger-data` → Ledger
  Processor Lambda → RDS
- **backfill:** `backfill-runner` (production) or `backfill-bench` (benchmark) CLI → same `process_ledger` call → RDS

Keeping the write-path identical means backfill and live ingest produce
byte-for-byte the same rows for a given ledger, and the replay-safe
derived-state guards work without special-casing.

### 6.3 Backfill Scope and Execution Model

- scope: from Soroban mainnet activation in late 2023 to the present
- batched in configurable ledger ranges; parallel only on non-overlapping
  ranges that preserve deterministic replay semantics
- one-time Phase 1 process; live ingestion continues in parallel; live-
  derived state remains authoritative for the newest ledgers
- no production infrastructure for backfill: no Fargate task, no ECS task
  definitions, no EventBridge schedule. The CLI runs on-demand from an
  operator's workstation

## 7. Worker Responsibilities

### 7.1 Ledger Processor

The Ledger Processor is the primary ingestion worker.

Its responsibilities are:

- consume S3 PutObject-triggered ledger artifacts
- parse and decode canonical XDR payloads
- treat ledger sequence as the canonical ordering key for writes
- extract structured explorer data
- write chain data and derived state to PostgreSQL
- keep replay of the same ledger idempotent
- prevent stale backfill writes from overwriting newer live-derived state

The Ledger Processor is the only Lambda worker in the indexing pipeline. It turns raw
ledger-close artifacts into first-class explorer records. If event enrichment (human-readable
interpretations of swap, transfer, mint, and burn patterns) is needed in the future, it will
be done inline within the Ledger Processor rather than in a separate Lambda.

## 8. Operational Characteristics

### 8.1 Normal Operation

The source design states the normal live path as:

```text
Galexie (ECS Fargate) -> S3 (~5-6 s per ledger)
                       -> Lambda Ledger Processor (~<10 s from ledger close to DB write)
```

This sets the baseline expectation for ingestion freshness.

### 8.2 Restart and Failure Recovery

The pipeline currently assumes:

- **Galexie restart recovery**: Galexie is checkpoint-aware and resumes from the last
  exported ledger automatically
- **Ledger Processor failure recovery**: Lambda retries S3-triggered processing
  automatically
- **Permanent processing failure**: failed files remain in S3 and can be replayed by
  re-triggering the Lambda with the S3 key
- **Replay safety and ordering**: immutable ledger-scoped writes are committed
  transactionally per ledger, and derived-state updates are monotonic by ledger sequence so
  older batches cannot regress newer state

These are core reliability assumptions of the ingestion architecture.

### 8.3 Schema and Protocol Change Handling

Operationally, the pipeline is also responsible for staying aligned with schema and protocol
changes.

The documented assumptions are:

- schema migrations are versioned, managed via AWS CDK, and run before deploying new Lambda
  code
- protocol changes affecting `LedgerCloseMeta` are handled by bumping the
  pinned Rust `stellar-xdr` crate version (per ADR 0004); the JS
  `@stellar/stellar-sdk` is used only on the frontend
- protocol upgrades are infrequent and announced in advance

### 8.4 Open-Source Redeployability

The source design explicitly assumes that the full infrastructure and ingestion pipeline can
be redeployed by third parties in a fresh AWS account.

For the indexing pipeline, that means:

- no hidden dependency on internal-only ingestion services
- no hidden dependency on external explorer APIs
- a fully reproducible Galexie -> S3 -> Lambda -> RDS flow

## 9. Boundaries and Delivery Notes

### 9.1 Boundary with Other Parts of the System

Responsibility split should remain clear:

- `apps/indexer` owns ingestion entrypoints and live/backfill pipeline behavior
- `apps/api` reads indexed data and does not perform primary ingestion
- `apps/web` consumes backend responses and does not parse canonical ledger artifacts

### 9.2 Workspace and Delivery Model

Within the current workspace direction documented in the repository:

- infrastructure deploys the runtime components
- application/runtime code is expected to live under `apps/indexer`, `apps/api`, and
  related packages
- infrastructure rollout is handled through AWS CDK and GitHub Actions

### 9.3 Current Workspace State

The repository currently documents the intended indexing pipeline shape but does not yet
contain the final production implementation of Galexie orchestration or the Ledger Processor.

That is expected. This document should serve as the detailed reference for future indexing
implementation planning, while
[`technical-design-general-overview.md`](../technical-design-general-overview.md) remains
the primary source of truth.
