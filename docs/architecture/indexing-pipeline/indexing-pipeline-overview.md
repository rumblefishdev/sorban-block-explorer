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
block explorer's own structured ClickHouse data model on the Hetzner box (per
[ADR 0044](../../../lore/2-adrs/0044_clickhouse-pilot-parallel-store.md)
and [ADR 0045](../../../lore/2-adrs/0045_clickhouse-as-prod-store.md);
task 0241 cut the live-tail indexer over from the legacy PG-on-RDS target).

Its purpose is to ingest chain data once, materialize explorer-facing records, and keep the
API and frontend independent from third-party explorer services or direct chain parsing at
request time.

This document covers the target design of the indexing pipeline only. It does not redefine
frontend behavior, backend transport contracts, or the detailed XDR parsing/storage model
except where those are needed to explain pipeline responsibilities.

This document describes the production ingestion model in its post-0241
(hard-swap PG → CH) form. Sections marked "intent" or "future" continue to
describe forward-looking design; the present-tense sections reflect the
implementation as of the live-tail cutover.

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the main
overview document takes precedence. This file is an indexing-pipeline-focused refinement of
that source, not an independent redesign.

## 2. Architectural Role

The indexing pipeline sits between canonical Stellar data sources and the explorer's owned
ClickHouse schema (hosted on the Hetzner AX52 box; Lambdas reach it via the Caddy mTLS
reverse proxy at `ch.sorobanscan.rumblefish.dev`).

Its role is to:

- stream or backfill canonical ledger data into the system
- parse `LedgerCloseMeta` payloads into structured explorer records
- persist those records into Hetzner ClickHouse via the Caddy mTLS reverse proxy
  (per-service client certs map to per-service CH users; see
  [`security/clickhouse-rbac.md`](../security/clickhouse-rbac.md))
- derive higher-level explorer entities such as contracts, accounts, assets, NFTs, and
  liquidity pools from canonical ledger artifacts
- make all normal backend and frontend reads depend on the explorer's own database rather
  than on external APIs

The pipeline is intentionally not a public API surface. It is an internal ingestion and
materialization boundary.

## 3. Pipeline Topology

### 3.1 End-to-End Flow

The indexing pipeline is a fixed event-driven chain. Post-0241 (live tail) it
ends at ClickHouse on Hetzner via the mTLS proxy:

```text
Stellar Network peers / history archives
  -> Galexie on ECS Fargate (eu-central-1)
  -> S3 bucket: stellar-ledger-data
  -> SNS topic: {env}-ledger-events    (S3 ObjectCreated notification; fan-out, task 0306)
  -> SQS queue: ledger-ingest          (SNS subscription, rawMessageDelivery=true)
  -> Lambda: Ledger Processor (eu-central-1, ARM64, out-of-VPC; SQS event-source-mapping)
  -> Caddy mTLS reverse proxy (Hetzner AX52)
  -> ClickHouse 26.x (Hetzner AX52)
```

The S3 → SNS → SQS → Lambda hop (not a direct S3 → Lambda trigger) is deliberate
(task 0241), and the SQS message is treated as a **content-free doorbell**, not
as "process this object":

- S3 `ObjectCreated` → the `{env}-ledger-events` SNS topic → the `ledger-ingest`
  SQS queue (via an SNS subscription with `rawMessageDelivery=true`). The message
  body is ignored by the Lambda either way — the doorbell is content-free, so the
  SNS-envelope-vs-raw body shape does not affect ingestion. `rawMessageDelivery`
  is set so the SQS body stays byte-identical to the legacy direct `S3 → SQS`
  shape and matches what the prices-api consumer expects (it _does_ read the S3
  object key from the body). The SNS topic exists to fan the same doorbells out
  to a second tenant (prices-api, same AWS account) on its own queue — S3 permits
  only one destination per overlapping `event + suffix`, so the topic replaces
  the direct wiring (task 0306).
- Each invocation runs a **reconcile**: read the durable cursor
  `max(sequence)` from CH, then persist the **contiguous** run of ledgers from
  `max + 1` upward — deriving each object key from the ledger number (Galexie's
  one's-complement-hex scheme, `files_per_partition = 64000`) and HEAD-checking
  S3 — stopping at the **first gap** (next ledger not yet on S3) or at a
  per-invocation **time budget** (540 s, under the 600 s function timeout).
- This guarantees **ascending, gapless processing without FIFO**: order comes
  from the cursor + S3 contents, not SQS delivery order (which is unordered).
  It is correct **only** at `reservedConcurrentExecutions = 1` (two concurrent
  reconciles would race the cursor) — load-bearing, not a preference.
- A reconcile that hits a gap or the time budget acks the doorbell; a hard
  CH/S3 failure fails the doorbell → SQS redelivers (per `maxReceiveCount`) →
  DLQ (`ledger-processor-dlq`), recoverable via SQS redrive-to-source.
- The S3 → SNS → SQS notification stays wired even when the Lambda is paused
  (`indexerLambdaConcurrency = 0`), so a paused indexer still captures events
  durably (visible `ApproximateNumberOfMessages`, multi-day retention) instead
  of dropping them.

A backlog drains across the stream of doorbells (one per S3 file ≫ the handful
of time-budget stops needed); the next doorbell always resumes from the
advanced `max`. **Strictly ordered bulk loads** (e.g. a snapshot-restore
re-run) still go through `crates/backfill-runner` (sequential by ledger
number), not this live path.

Historical backfill runs through `crates/backfill-runner` on operator
workstations and lands directly into the same Hetzner ClickHouse (the
3-way parallel-backfill merge of task 0228 completed at
`L_last_closed = 62,527,999`). See §6 for the backfill path.

### 3.2 Main Runtime Components

The pipeline depends on seven primary runtime components:

- **Galexie on ECS Fargate (eu-central-1)** for canonical ledger export
- **S3** for transient `LedgerCloseMeta` object storage
- **`{env}-ledger-events` SNS topic** as the fan-out point so S3 doorbells reach
  both the indexer's queue and a second tenant (prices-api) on its own queue
  (task 0306)
- **`ledger-ingest` SQS queue** as the durable buffer between the topic and the
  Lambda (S3 `ObjectCreated` → SNS → SQS with `rawMessageDelivery`; redrive →
  `ledger-processor-dlq` after `maxReceiveCount`)
- **Ledger Processor Lambda** for event-driven parsing and persistence (ARM64,
  out-of-VPC, AWS Parameters and Secrets Lambda Extension for mTLS cert
  retrieval; driven by an SQS event-source-mapping with
  `ReportBatchItemFailures`)
- **Caddy reverse proxy on Hetzner** terminates mTLS, validates the client
  cert chain, maps the cert CN → CH user via `CLICKHOUSE_CN_USER_MAP`, and
  forwards to the local CH on loopback (per
  [`security/clickhouse-rbac.md`](../security/clickhouse-rbac.md))
- **ClickHouse 26.x on Hetzner AX52** as the explorer's owned storage target

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

For each arriving S3 object the Lambda follows the path below. Per-ledger CH
atomicity comes from the one-shot `persist_ledger_clickhouse` wrapper
(`crates/db-clickhouse/src/persist.rs`): open `PartitionWriter` → `write_ledger`
→ `commit` per ledger. The `ledgers` row is the commit marker — a mid-ledger
failure leaves no `ledgers` row for that ledger, the Lambda returns Err, S3
redelivers the whole batch, and `ReplacingMergeTree` collapses any orphan
non-`ledgers` rows on the next background merge. Already-committed earlier
ledgers in the same batch keep their `ledgers` rows; redelivery may produce
duplicate `ledgers` rows for those sequences (see §5.3 note).

1. download and decompress the XDR file from S3
2. parse `LedgerCloseMeta` using the Rust `stellar-xdr` crate (ADR 0004) and
   extract the shared canonical data via `crates/xdr-parser` —
   `parse_ledger()` is pure and shared with the backfill path
3. for each ledger in the batch: call
   `db_clickhouse::persist::persist_ledger_clickhouse(&client, &parsed.*)`
   — the same wrapper backfill's `Sink::persist_ledger` fallback drives.
   It stages rows via
   `db_clickhouse::persist::stage::prepare_with_sac_overrides`, opens a
   one-shot `PartitionWriter`, streams the staged rows, and commits. The
   hybrid-key strategy keeps three high-fan-out hubs (`accounts`,
   `soroban_contracts`, `transactions`) on a deterministic surrogate
   `Int64 id` derived from the StrKey / hash via cityhash; the other 12
   tables use natural composite keys with `LowCardinality(String)`
   dictionary encoding (per
   [ADR 0044](../../../lore/2-adrs/0044_clickhouse-pilot-parallel-store.md))
4. retry envelope per ledger: transient errors retry with backoff
   `[50, 200, 800] ms`. `Error::Network` / `Error::TimedOut` always
   retry. `Error::BadResponse` is classified by a **denylist**, not an
   allowlist — the `clickhouse` 0.15 crate carries the raw response
   body verbatim (it does not expose the HTTP status separately), so a
   5xx with a body would slip past a `starts_with("502 ")` allowlist
   and trip the DLQ. We therefore retry every `BadResponse` **except**
   definitively permanent ones: HTTP 4xx prefixes and known CH semantic
   exception codes (`UNKNOWN_TABLE`, `TYPE_MISMATCH`,
   `CANNOT_PARSE_*`, `SYNTAX_ERROR`, auth codes, …). Rationale: the
   live-tail INSERT is deterministic, so a non-permanent error is
   almost always infra turbulence; availability beats a marginal retry
   cost on an unrecognised permanent error (3 wasted retries → same DLQ
   terminal state). See `is_retryable_bad_response` /
   `CH_PERMANENT_CODES` in `crates/indexer/src/handler/mod.rs`
5. emit a CW custom metric `LastProcessedLedgerSequence` after each
   ledger's commit — fire-and-forget; failures are warn-logged but do not
   abort the batch
6. **enrichment SQS publish is stubbed** (task 0241) — re-enablement
   awaits the paired CH-aware rewrite of producer + `enrichment-worker`
   write path. See [`enrichment.md`](./enrichment.md) for the design intent
   The per-table column-by-column write order, FK dependencies, and the
   `liquidity_pools` orphan sentinel handling (per
   [ADR 0041](../../../lore/2-adrs/0041_lp-positions-orphan-handling-state-filter-and-sentinel-pool.md))
   are documented at code-level in the CH writer module
   (`crates/db-clickhouse/src/persist/writer.rs`) and in the schema file
   (`crates/db-clickhouse/schema/init.sql`). The indexer writes no aggregate at
   all: supply/holders are recomputed from `balances` by the refreshable
   `balance_aggregates_mv`, and the dead `assets.holder_count` /
   `assets.total_supply` columns were dropped in task 0310.

**Soroban AMM pools** (ADR 0058, task 0374) ride the same pass with no extra
step: `parse_ledger()`'s event sweep detects router `add_pool` registrations
(`detect_pool_registrations`) and its ledger-entry-change walk extracts pool
state (`extract_plane_pool_data` for fungible pools' plane `PoolData`,
`extract_pool_instances` for pool instances — concentrated reserves +
`TokenShare` / `Plane` / `Router`). Staging turns these into
`liquidity_pools` rows (`pool_kind = 1`, whole-row registration — never
partially updated on the RMT), `pool_state_changes` rows (one per
`(pool, ledger)`) and `pool_instance_state` rows (side table, `asset_sac`
pattern). An unparseable fee or reserve REFUSES the row with
`tracing::error!` rather than writing a plausible default.

Two staging rules are load-bearing here, both because a registration event
names its pool in a payload the emitter chooses freely:

- **A registration is corroborated before it becomes a row.** The named pool's
  own instance storage — where the pool contract is the ledger-authenticated
  owner — must declare that emitter as its `Router`. The instance is written
  in the SAME transaction as `add_pool`, so the corroborating fact is always
  in the same parse output. Without this, any contract could emit an
  `add_pool` naming a REAL pool and replace its registry row wholesale, since
  `liquidity_pools` is an RMT keyed on `pool_id` and versioned by ledger.
  One documented exception: an instance with no `Router` key at all (five
  older deployments, 23 pools) is accepted UNVERIFIED with a warn, and only
  when the instance is CREATED in the registering ledger — every genuine
  registration creates it there (497/497 measured), while a merely-touched
  instance is the induced-forgery signature and is refused. A missing key is
  an older contract version, not evidence of a forgery.
  The Soroswap arm (task 0518) applies the same discipline with no
  UNVERIFIED case at all: a `new_pair` stages only when the pair's own
  instance names the emitting factory (DataKey 4) AND was created in the
  registering ledger — validated on raw ledgers across three eras.
  The config-factory arm (task 0518, third adapter) has no back-pointer to
  check — the pool records no factory anywhere — so its corroboration is
  the created gate plus the pool's own full `CONFIG` written in the
  registering ledger (every registry fact comes from that CONFIG, none from
  the event); validated on the family's entire registration population.
  Its per-operation state rows are self-stamped like the pair family's, and
  its `pool_instance_state` row stages ONLY when the transaction wrote
  CONFIG: the table is RMT whole-row on `pool_id`, and a config-less
  TotalShares write would clobber `share_token_id` to 0.
- **Both reserve writers are folded together** before insert. The plane arm
  and the concentrated-instance arm can each emit a row for the same
  `(pool, ledger)`, and the parser-side folds cannot see each other; a
  version-less RMT would then keep an arbitrary intra-ledger image. The fold
  is the cross-writer twin of `dedup_final_pool_snapshots` (lore 0356).

The historical 15-step PG flow (atomic per-ledger `BEGIN/COMMIT`) was removed with
Postgres (task 0244); its ordering rationale is preserved in
[ADR 0027](../../../lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md).
The schema design rationale it relied on (BYTEA hashes, surrogate IDs, etc.)
carries forward to the ClickHouse schema per ADRs 0024 / 0026 / 0029 / 0030 / 0031.

### 5.3 Write Target

The live ingestion path writes directly to the explorer's owned ClickHouse
schema on Hetzner. That write includes both:

- low-level structured explorer records (`ledgers`, `transactions`,
  `operations_appearances`, `transaction_participants`, and the appearance
  indexes `soroban_events`, `soroban_invocations_appearances`)
- derived explorer-facing state (`accounts`, `soroban_contracts`,
  `wasm_interface_metadata`, `assets`, `nfts`, `nft_ownership`,
  `nfts_pending`, `nft_ownership_pending`, `liquidity_pools`,
  `liquidity_pool_snapshots`, `lp_positions`, `account_balances_current`)

The full table inventory (17 + 2 quarantine + 1 dictionary = 20 schema
objects) is in `crates/db-clickhouse/schema/init.sql`.

The presence indexes `transaction_participants` and `operation_asset_appearances`
are fed from **two** sources: classic operation bodies, and decoded Soroban token
events. Per lore task
[0383](../../../lore/1-tasks/active/0383_FEATURE_l2-soroban-event-token-flow-decode/README.md)
the staging event loop decodes SEP-41 / CAP-67 `transfer` / `mint` / `burn` /
`clawback` events (`derive_token_event`, see xdr-parsing overview §5.6) and
registers their `from` / `to` as account participants plus — for SAC-wrapped
classic/native assets — the moved asset (`"native"` → `NATIVE_ASSET_ID`).
`transaction_participants` stays pure presence.

`operation_asset_appearances` is pure presence. The `net_settled` value column
(task 0393) was REMOVED on 2026-09-04 — the per-(tx, asset) aggregate carried no
direction and no account. The reducer that produced it
(`persist::stage::ledger_deltas_net_settled` over
`xdr_parser::ledger_balance_deltas`) is KEPT: it reads the authoritative LEDGER
balance changes, which is the input the lossless replacement needs.

Per-ledger replay safety: every state table is `ReplacingMergeTree(version)`
keyed on a column whose value monotonically reflects the latest observation
(`last_seen_ledger`, `last_updated_ledger`, `current_owner_ledger`,
`wasm_uploaded_at_ledger`). Replay → re-stage same rows → the next background
merge collapses the duplicates to the latest version. Fact tables use plain
`ReplacingMergeTree` over `ORDER BY` (same dedup contract, no version column
needed because the rows themselves are immutable per `(tx_hash, op_idx)` etc.).

List and partition-pruned reads from the API Lambda hit this CH schema directly
once that Lambda's read-path migration lands (the API still queries PG in the
post-0241 "stale window" — task 0243). Heavy-field endpoints (E3
`/transactions/:hash`, E14 `/contracts/:id/events`) continue to fetch raw
`.xdr.zst` from the public Stellar ledger archive and re-parse at request
time per ADR 0029 — that is a **read-path** dependency, not an ingest-path one;
the indexing pipeline itself never calls the public archive.

## 6. Historical Backfill Flow

### 6.1 Backfill Source and Runtime

Per [ADR 0010](../../../lore/2-adrs/0010_local-backfill-over-fargate.md),
historical backfill runs as a **local CLI tool** (`crates/backfill-runner`) on a
developer workstation. It streams from Stellar's **public history archives** (the same
archives Horizon used for `db reingest`) and writes directly to ClickHouse on
Hetzner. The historical 3-way parallel-backfill
merge of [task 0228](../../../lore/1-tasks/archive/0228_FEATURE_parallel-backfill-merge-and-validation/README.md)
completed at `L_last_closed = 62,527,999` with zero gaps and ≤ 0.01 %
mismatch against Horizon on 980 stratified samples.

### 6.2 Shared Code Path, Shared Storage

Live ingestion and backfill now write to the same Hetzner CH instance. The
indexer crate's `parse_ledger` half is shared; the persist half is two-flavored:

- **live (Lambda):** Galexie (ECS Fargate eu-central-1) → S3
  `stellar-ledger-data` → `ledger-ingest` SQS → Ledger Processor Lambda →
  Caddy mTLS → Hetzner CH
- **backfill (CLI):** `backfill-runner` → same parse path
  → `db_clickhouse::persist::PartitionWriter` → Hetzner CH

The CH writer module (`crates/db-clickhouse/src/persist/writer.rs`) drives
backfill via **partition-aligned streaming inserts** — open writer →
`write_ledger × N` → `commit` — so backfill emits one `INSERT` per CH table
per 64k-ledger partition (≈ 3 100 `INSERT` statements over an 11M-ledger
backfill, well inside merger comfort). The live-tail indexer reuses the same
parse half but drives the persist half **per-ledger** via the one-shot
`persist_ledger_clickhouse` wrapper (open writer → write_ledger → commit per
ledger) — fewer parts per partition than the long-running backfill writer,
but appropriate to the per-S3-event Lambda model and to per-ledger commit
isolation under retry.

The `backfill-runner` CH writer plumbing shipped in task
[0205](../../../lore/1-tasks/archive/0205_FEATURE_backfill-runner-clickhouse-target-flag.md);
the real partition-aligned writer landed in task
[0206](../../../lore/1-tasks/archive/0206_FEATURE_clickhouse-persist-real-inserts/README.md).
The legacy Postgres sink was removed in task 0244 — the runner is
ClickHouse-only. Full design plus the `soroban_events` ADR 0044 §Decision §4a
unfold are documented in
[`docs/architecture/database-schema/clickhouse-pilot.md#writers`](../database-schema/clickhouse-pilot.md#writers).

CH-target runs additionally accept an optional `--soroban-rpc-url` /
`SOROBAN_RPC_URL` flag (task
[0214](../../../lore/1-tasks/active/0214_FEATURE_ch-initial-snapshot-account-state.md))
that turns on the **initial-snapshot mechanism** for account state.
After the per-ledger ingest loop finishes, the runner discovers
skeleton accounts (`accounts FINAL WHERE sequence_number = 0`)
referenced by `transaction_participants` in the window, fetches
their live `AccountEntry` via Soroban RPC `getLedgerEntries`, and
tops up `accounts` + `account_balances_current` so they no longer
look like skeletons. Without the flag the bootstrap step is skipped
(participants-driven skeleton rows persist as-is). The mechanism
closes the 2026-05-12 CH-pilot audit §E06 gap and is documented in
[`docs/architecture/database-schema/clickhouse-pilot.md#state-side-ingestion-initial-snapshot-mechanism`](../database-schema/clickhouse-pilot.md#state-side-ingestion-initial-snapshot-mechanism).

The same `--soroban-rpc-url` flag drives the **`balance-seed`** one-shot pass
(task [0331](../../../lore/1-tasks/active/0331_FEATURE_soroban-token-supply-holders-event-fold/README.md))
for bespoke Soroban-token (`asset_type = 3`) per-holder balances. The live
parser writes the unified `balances` table only when it observes a
`ContractData` `Balance(Address)` change, so dormant holders are absent and
`total_supply` (`sum(amount)`) + `holder_count` (`countIf(amount > 0)`)
under-count. `balance-seed` enumerates each type-3 token's holder candidates
from its `soroban_events` topics/data (the event SET — value comes from ledger
STATE, never an event-fold), reads their current `Balance(Address)` entries via
`getLedgerEntries`, and upserts `balances`. Supply is then the single
`balance_aggregates` `sum(amount)` (task 0331 Option A — no per-token
`TotalSupply` key read; a mint always credits a holder balance, contract
treasuries summed because holders include `C…`, so the sum equals real supply;
residue = TTL-archived tail + true rebasing). It reads CURRENT chain state, so the snapshot is correct
regardless of indexer lag; live ingest supersedes it on catch-up
(`ReplacingMergeTree` by `last_updated_ledger`). CH-only, idempotent, `--dry-run`
supported.

The historical gap for these event-driven presence rows — the live hook only
writes them for new ledgers, and event-derived asset presence never existed for
any verb — was closed by the **`soroban-token-flow-backfill`** one-shot pass (task
[0383](../../../lore/1-tasks/active/0383_FEATURE_l2-soroban-event-token-flow-decode/README.md)),
run on prod 2026-07-16 and **removed in lore 0425**. It re-derived participant +
asset rows from `soroban_events` with the SAME `derive_token_event` the live path
uses (the surrogate hashing is `cityhash_102_128`, not CH SQL's `cityHash64()`, so
the decode must run in Rust to stay bit-identical). History is closed; live ingest
is now the only writer of these rows.

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

- consume ledger artifacts via the `ledger-ingest` SQS queue (fed by S3
  `ObjectCreated`)
- parse and decode canonical XDR payloads
- treat ledger sequence as the canonical ordering key for writes
- extract structured explorer data
- write chain data and derived state to Hetzner ClickHouse via the Caddy
  mTLS reverse proxy (cert bundle fetched at cold start from AWS Secrets
  Manager via the Parameters and Secrets Lambda Extension)
- keep replay of the same ledger idempotent (per-ledger commit-marker —
  the `ledgers` row is the last insert per ledger — plus
  `ReplacingMergeTree` collapse on background merge for the 17 RMT state
  tables)
- prevent stale backfill writes from overwriting newer live-derived state
  (handled by the `ReplacingMergeTree(version_column)` choice — version
  columns are ledger-derived and monotonic)

The Ledger Processor is the only Lambda worker on the **ingestion path** — it turns raw
ledger-close artifacts into first-class explorer records. **Inline-eligible** event enrichment
(human-readable interpretations of swap / transfer / mint / burn patterns) stays inside the
Ledger Processor; the criterion is "derivable purely from the processed ledger".

A second worker — the SQS-driven **enrichment Lambda 2** introduced in task 0191 and
documented in [`enrichment.md`](./enrichment.md) — runs **off** the ingestion path and handles
work that fails the inline criterion: oracle lookups (USD prices), per-row HTTP fetches
(SEP-1 issuer TOML, NFT `token_uri()`), and any expensive or long-running enrichment that
would push the Ledger Processor past its per-ledger budget. Lambda 2 consumes SQS messages
emitted by the Ledger Processor after each ledger commit and writes the result back to typed
columns; the two Lambdas share neither code path nor invocation lifecycle.

Allocation rule (codified by [ADR 0043](../../../lore/2-adrs/0043_field-allocation-rule.md)):

- **On-chain + cheap** → inline in the Ledger Processor.
- **Off-chain (HTTP / oracle / per-row RPC) + needed by list endpoints** → Lambda 2 (typed-column write).
- **Detail-only off-chain fields** → runtime type-2 fetch in the API handler (no DB column).

## 8. Operational Characteristics

### 8.1 Normal Operation

The normal live path is:

```text
Galexie (ECS Fargate eu-central-1) -> S3 (~5-6 s per ledger)
                                    -> ledger-ingest SQS
                                    -> Lambda Ledger Processor
                                    -> Caddy mTLS (Hetzner)
                                    -> ClickHouse 26.x
                                    (total ~<10 s from ledger close to DB write)
```

This sets the baseline expectation for ingestion freshness. The
`live-tail-cutover.md` runbook B-2 step expects `< 30 s` lag (Horizon tip vs
CH max sequence) at steady state.

### 8.2 Restart and Failure Recovery

The pipeline currently assumes:

- **Galexie restart recovery**: Galexie is checkpoint-aware and resumes from the last
  exported ledger automatically
- **Ledger Processor failure recovery**: the doorbell reconcile resumes from
  the durable cursor (`max(sequence)`), so recovery needs no per-event state.
  A reconcile that hits a gap or its time budget acks the doorbell; a hard
  CH/S3 failure reports it via `ReportBatchItemFailures` → SQS redelivers it
  after the visibility timeout, up to `maxReceiveCount`. In-band transient CH
  errors are retried first within the `[50, 200, 800] ms` envelope before the
  doorbell is failed back to SQS.
- **Permanent processing failure**: after `maxReceiveCount` the doorbell lands
  in `ledger-processor-dlq` (the underlying ledger files stay in S3). Recover
  by SQS redrive-to-source from the DLQ back to `ledger-ingest` (native SQS
  operation) once the cause is fixed — any doorbell re-triggers the same
  cursor-based reconcile.
- **Replay safety and ordering**: the reconcile advances strictly ascending
  from `max + 1` and stops at the first S3 gap, so a stalled predecessor
  blocks its successors — no out-of-order persistence. Per-ledger CH writes
  use the commit-marker pattern (`persist_ledger_clickhouse` writes the
  `ledgers` row last); a mid-ledger failure leaves no `ledgers` row, so the
  next reconcile resumes at exactly that ledger without reprocessing the
  committed ones. `ReplacingMergeTree(version_column)` still collapses any
  orphan non-`ledgers` rows on the next background merge, and derived state is
  monotonic by ledger sequence — `ledgers` itself stays plain MergeTree, so
  correctness queries use `count(DISTINCT sequence)` (runbook B-2)

These are core reliability assumptions of the ingestion architecture.

### 8.3 Schema and Protocol Change Handling

Operationally, the pipeline is also responsible for staying aligned with schema and protocol
changes.

The documented assumptions are:

- schema migrations are versioned, managed via AWS CDK, and run before deploying new Lambda
  code
- protocol changes affecting `LedgerCloseMeta` are handled by bumping the
  pinned Rust `stellar-xdr` crate version (per ADR 0004); the frontend consumes
  typed API responses via OpenAPI-generated TS client (task 0096).
- protocol upgrades are infrequent and announced in advance

### 8.4 Open-Source Redeployability

The source design explicitly assumes that the full infrastructure and ingestion pipeline can
be redeployed by third parties in a fresh AWS account.

For the indexing pipeline, that means:

- no hidden dependency on internal-only ingestion services
- no hidden dependency on external explorer APIs
- a fully reproducible Galexie -> S3 -> Lambda -> CH flow (the CH side is
  Ansible-managed on a generic Linux box; `infra-hetzner/` is provider-neutral
  apart from the bare-metal provisioning bit)

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
