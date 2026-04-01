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
- derive higher-level explorer entities such as contracts, accounts, tokens, NFTs, and
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

For each arriving ledger artifact, the current pipeline model is:

1. download and decompress the XDR file
2. parse `LedgerCloseMeta` using `@stellar/stellar-sdk`
3. extract ledger header data
4. extract transaction summaries and raw XDR artifacts
5. extract operation details
6. extract Soroban invocations
7. extract CAP-67 contract events
8. extract contract deployments from `LedgerEntryChanges`
9. extract account state snapshots from `LedgerEntryChanges`
10. detect token contracts, NFT contracts, and liquidity pools from deployment/event-derived patterns
11. write all resulting records to RDS PostgreSQL

### 5.3 Write Target

The live ingestion path writes directly to the explorer's owned PostgreSQL schema.

That write includes both:

- low-level structured explorer records such as ledgers, transactions, operations,
  invocations, and events
- derived explorer-facing state such as accounts, tokens, NFTs, and liquidity pools

The API layer reads only from this persisted result, not from live ingestion memory or raw
S3 objects.

## 6. Historical Backfill Flow

### 6.1 Backfill Source

Historical backfill uses Stellar public history archives.

A separate ECS Fargate task reads historical data and emits the same `LedgerCloseMeta`
artifacts into the same S3 bucket used by the live flow.

### 6.2 No Separate Parse Path

The source design explicitly avoids a separate processing implementation for backfill.

Backfill and live ingestion converge at the same handoff boundary:

- same XDR artifact shape
- same S3 destination
- same Ledger Processor Lambda
- same downstream database write path

This is important because it keeps the ingestion contract uniform and reduces divergence
between historical and live processing logic.

### 6.3 Backfill Scope and Execution Model

Current documented assumptions are:

- backfill scope starts from Soroban mainnet activation in late 2023
- backfill runs in configurable ledger-range batches
- batches may run in parallel only when they own non-overlapping ledger ranges and preserve
  deterministic replay semantics
- backfill runs as a one-time Phase 1 process while live ingestion continues in parallel,
  with live-derived state remaining authoritative for the newest ledgers

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
- protocol changes affecting `LedgerCloseMeta` are handled by updating
  `@stellar/stellar-sdk` XDR support
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
