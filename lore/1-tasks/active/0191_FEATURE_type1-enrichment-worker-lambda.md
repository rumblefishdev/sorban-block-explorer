---
id: '0191'
title: 'Type-1 enrichment live path (icon only): SQS-driven worker for assets.icon_url'
type: FEATURE
status: active
related_adr: ['0029']
related_tasks: ['0124', '0187', '0188']
tags:
  [
    priority-high,
    effort-medium,
    layer-indexer,
    layer-backend,
    layer-infra,
    milestone-2,
  ]
milestone: 2
links:
  - lore/1-tasks/archive/0188_FEATURE_sep1-fetcher-and-assets-details-enrichment.md
  - lore/1-tasks/archive/0187_REFACTOR_runtime-enrichment-module-rename.md
  - lore/2-adrs/0029_archive-runtime-fetch.md
history:
  - date: '2026-05-05'
    status: backlog
    who: karolkow
    note: 'Task drafted after 0188. Live-path-only, icon-only MVP. Backfill deferred to Future Work (no separate task spawned yet); LP analytics deferred (track separately under 0125).'
  - date: '2026-05-05'
    status: active
    who: karolkow
    note: 'Activated for implementation. Branch feat/0191_type1-enrichment-worker-lambda cut from develop with feat/0188 merged in for the SEP-1 fetcher base.'
---

# Type-1 enrichment live path (icon only): SQS-driven worker for `assets.icon_url`

## Summary

Introduce the **live-path** half of type-1 enrichment for the **single
column `assets.icon_url`**. Type-1 = HTTP fetches that write DB columns
instead of fetching per request. The MVP scope is deliberately tiny:
prove the SQS-driven producer/worker shape end-to-end on one column,
add more kinds later.

```
crates/indexer (Galaxy live ingester Lambda)
        │
        │  on each new asset row
        ▼
   SQS enrichment-queue  ─── DLQ + alarm
        │
        ▼
crates/enrichment-worker (Lambda, this task)
        │
        │  HTTPS GET https://{home_domain}/.well-known/stellar.toml
        ▼
   UPDATE assets SET icon_url = …
```

Out-of-scope-but-related:

- **Backfill** (operator-driven CLI for rows pre-dating the producer)
  — deferred. Captured in "Future Work" below; no separate task
  spawned yet. Will be picked up after 0191 ships and proves the
  live path.
- **LP analytics columns** (`tvl_usd`, `volume_24h_usd`,
  `fee_revenue_24h_usd`) — deferred. The original LP task `0125` stays
  in backlog as the place to track that work, and will reuse the
  machinery built in 0191 (queue, worker, shared lib) when it's picked
  up. Not superseded by this task.

This task introduces the new shared lib `crates/enrichment-shared`
(used by future backfill / kinds), the new
`crates/enrichment-worker` Lambda binary, the SQS produce in
`crates/indexer`, and the CDK changes for the queue + worker Lambda.
**No DB migration is needed** — `assets.icon_url` already exists.

## Status: Active

**Current state:** Implemented. Shared crate, worker Lambda, indexer
producer, and CDK wiring landed in commit `25c93c9`. Open items: two
operator-driven acceptance items remain unchecked (`cdk diff` clean +
DLQ poison-message verification) — see Acceptance Criteria.

## Context

### Where this fits

After 0187 (`runtime_enrichment` module rename) and 0188 (SEP-1 type-2
fetcher) we have:

```
crates/api/src/runtime_enrichment/
├── stellar_archive/   ← S3 archive XDR reread (ADR 0029)
└── sep1/              ← stellar.toml HTTPS fetcher (0188)
```

That's **type-2** — per-request, in-process LRU cache, fail-soft, no DB
writes. Right architecture for fields that are large/rare (e.g. asset
description from SEP-1, only shown on the detail page).

**Type-1** is the other half: fields that need to be persisted because
they're displayed on list endpoints, used in filters/joins, or are
stable enough that paying the HTTP cost on every request is wasteful.

`assets.icon_url` is the canonical type-1 case: shown on every asset
list row (the logo column), so paying SEP-1 fetch cost per request is
unacceptable; rare to change, so a one-time fetch + persisted column
is the right shape.

### Why SQS + worker (not cron + worker)

The indexer (Galaxy Lambda) sees new assets in real time. It already
commits the DB row; emitting an SQS message afterwards gives the
worker a precise list of rows that need enrichment, with no scanning.
Cron + scan would either be wasteful (scan whole table) or require
state ("last seen ledger") on top of the DB. SQS is the simpler path:
producer knows exactly what's new.

### Worker always fetches and always writes

Every time the live worker receives a message it does the full
fetch + UPDATE from scratch. No `WHERE icon_url IS NULL`
short-circuit, no value-equality check. Duplicate messages cost a
redundant fetch + UPDATE; that cost is acceptable and keeps the
worker logic trivial.

### What this task supersedes

- `lore/1-tasks/active/0124_FEATURE_token-metadata-enrichment.md` —
  earlier draft for token metadata enrichment as a JSONB blob. The
  0188 SEP-1 fetcher delivered the type-2 metadata path; this 0191
  task delivers the type-1 icon path via worker. Mark 0124
  `superseded` by `['0188', '0191']` in this task's cleanup step.

`0125 (lp-price-oracle-tvl-volume)` is **not** superseded — LP
analytics is dropped from MVP and 0125 remains the placeholder. It
will be picked up later and will reuse the queue/worker/shared-lib
shipped here.

## Implementation Plan

### Step 1 — `crates/enrichment-shared` lib crate

New library crate. Move SEP-1 fetcher out of
`crates/api/src/runtime_enrichment/sep1/` into this shared crate so
the worker (this task) and any future enrichment consumer (backfill
CLI, additional kinds) can depend on it without a cyclic `api` dep.

```
crates/enrichment-shared/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── sep1/
    │   ├── mod.rs
    │   ├── fetcher.rs          ← Sep1Fetcher (moved from runtime_enrichment::sep1)
    │   ├── dto.rs
    │   ├── errors.rs
    │   └── validation.rs       ← validate_host (RFC 1035 + IP-literal reject)
    └── enrich_and_persist/
        ├── mod.rs
        ├── error.rs
        └── icon.rs             ← pub async fn enrich_asset_icon(pool, asset_id, fetcher) -> Result
```

The `enrich_asset_icon` function is the **single source of truth** for
"given this asset id, fetch its issuer's stellar.toml, extract the
matching `CURRENCIES[].image`, and write `icon_url`". Worker calls it
per SQS message; the future backfill CLI will call it per row from a
streaming SELECT; the live api never calls it (api uses the type-2
path for description / home_page only).

Behaviour:

1. `SELECT a.asset_code, iss.account_id, iss.home_domain FROM assets a LEFT JOIN accounts iss ON iss.id = a.issuer_id WHERE a.id = $1` — `home_domain` lives on `accounts`, not `assets`.
2. If `home_domain IS NULL` (or empty / whitespace-only after trim) → write `''` empty sentinel + `Ok(())` (asset has no issuer-published toml; nothing to fetch). Future re-run won't refetch.
3. Otherwise fetch SEP-1 toml via the existing `Sep1Fetcher` (reuses 0188's HTTP+cache+validation).
4. Walk `CURRENCIES[]`, find row where `(code, issuer)` matches; read `.image`.
5. `UPDATE assets SET icon_url = $1 WHERE id = $2` — unconditional overwrite, no `IS NULL` filter.
6. Return `Err` only on hard failures (DB error, malformed toml). Network timeouts / 5xx return `Err` so SQS retries; permanent 4xx (toml 404) write the `''` sentinel and return `Ok(())` so the message is acked.

Update `crates/api/src/runtime_enrichment/sep1/mod.rs` to re-export
from `enrichment_shared::sep1`, keeping the api crate's public surface
unchanged.

### Step 2 — `crates/enrichment-worker` Lambda binary

```
crates/enrichment-worker/
├── Cargo.toml          ← binary, depends on lambda_runtime, enrichment-shared, db
└── src/
    └── main.rs         ← lambda_runtime entry, parses SqsEvent, dispatches
```

Behaviour per invocation:

1. Receive `SqsEvent` (batch of N records, configured in CDK).
2. For each record, parse JSON message body: `{ kind: "icon", asset_id: i32 }`.
3. Match `kind`:
   - `"icon"` → `enrichment_shared::enrich_and_persist::icon::enrich_asset_icon(&pool, asset_id).await`
   - other → `tracing::warn!` + treat as success (ack the message; an unknown kind is a producer bug, retry won't fix it). Forward-compatible with future kinds.
4. On `Ok(())` — message ack'd, removed from queue.
5. On `Err(_)` — return error from handler → SQS will redeliver per
   `redrivePolicy.maxReceiveCount` (3 by default). After 3 failed
   deliveries the message moves to the DLQ.

No `WHERE … IS NULL` short-circuit (see "Worker always fetches and always writes").

### Step 3 — `crates/indexer` (Galaxy) SQS produce

Extend the live indexer Lambda. Wherever the indexer commits a new
asset row (`INSERT INTO assets ...`), also publish an SQS message:

```jsonc
{ "kind": "icon", "asset_id": 12345 } // asset_id is i32 (assets.id SERIAL)
```

**Open design point** (see Open Questions): publish inside the same
write transaction (couples indexer availability to SQS) versus publish
after commit (eventually consistent, requires a janitor for misses).
Lean toward "publish after commit + future janitor" — but that's
deliberately deferred to "Future Work".

Producer is `INSERT`-only — Galaxy does not currently update existing
rows, so no separate "row updated" code path is needed yet. When/if
that changes (e.g. a future "asset metadata changed" event), the
indexer can emit refresh messages from the same hook.

### Step 4 — CDK infra

- New SQS standard queue `enrichment-queue`.
- New SQS DLQ `enrichment-queue-dlq` with `redrivePolicy.maxReceiveCount=3`.
- 14-day retention on both queues.
- New CloudWatch alarm: `enrichment-queue-dlq` `ApproximateNumberOfMessagesVisible > 0` for 5 min → SNS topic (or whatever pager exists).
- New `enrichment-worker` Lambda function (Rust binary, same packaging as `indexer`).
- Event source mapping: `enrichment-queue` → `enrichment-worker`, `BatchSize=10`, `ReservedConcurrency=2`.
- IAM: worker needs `sqs:ReceiveMessage`, `sqs:DeleteMessage` on the queue, RDS Proxy access, SecretsManager for DB creds.
- Indexer Lambda gets `sqs:SendMessage` on the queue.

The queue is named generically (`enrichment-queue`) rather than
icon-specific so future kinds (LP analytics, prices, etc.) can re-use
the same infra without renaming.

### Step 5 — Cleanup

- `lore/1-tasks/active/0124_FEATURE_token-metadata-enrichment.md` — set `status: superseded`, `by: ['0188', '0191']`, move to `archive/`.
- `0125_FEATURE_lp-price-oracle-tvl-volume.md` — **leave in backlog**, no status change. It is the placeholder for the next type-1 kind once 0191 lands.
- ADR 0029 amendment (separate sub-task, not in this PR): update to describe the `runtime_enrichment` umbrella + the SQS-driven type-1 model. Spawn a backlog task for the ADR amendment so it doesn't slip.
- Lore index regen via `lore_generate-index`.

## Acceptance Criteria

- [x] `crates/enrichment-shared` lib crate builds, hosts SEP-1 fetcher + `enrich_asset_icon`.
- [x] `crates/api/src/runtime_enrichment/sep1/` re-exports from `enrichment-shared` (kept as thin shim so existing api-internal imports keep working unchanged).
- [x] `crates/enrichment-worker` Lambda binary builds, deployable, parses `SqsEvent`, dispatches `kind: "icon"` to `enrich_asset_icon`.
- [x] Worker writes are unconditional overwrites — duplicate messages succeed and update `icon_url` to whatever the source currently returns.
- [x] `crates/indexer` emits an SQS message for each newly inserted asset. `publish_for_extracted_assets` instrument span carries `kind="icon"` + `extracted` + `published` fields; per-id debug log carries `kind` + `asset_id`.
- [ ] CDK provisions the queue, DLQ, alarm, worker Lambda, event source mapping, and IAM. `cdk diff` clean on a stack synth.
- [ ] DLQ alarm verified by injecting a poison message in a non-prod env (worker fails, message moves to DLQ, alarm fires).
- [x] `0124` marked superseded and moved to archive in the same PR. (Task md updated with `status: superseded` + `by: ['0188', '0191']`; ready in working tree.)
- [x] **Docs updated** — `docs/architecture/database-schema/database-schema-overview.md` updated: §4.10 enrichment topology now describes the SQS-driven type-1 path (indexer producer + `enrichment-worker` Lambda + sentinel handling). `assets.icon_url` definition unchanged. Per ADR 0032.
- [x] **API types regenerated** — `npx nx run @rumblefish/api-types:generate` ran clean with no diff (the sep1 module move did not change the api crate's public schema surface).

## Out of Scope / Open Questions

Deferred — not part of this task, captured here so they're not lost.

- **Local backfill subcommand** — captured in "Future Work" below; no separate task spawned yet. Will become a `backfill-runner enrichment` subcommand when picked up.
- **LP analytics columns** (`tvl_usd`, `volume_24h_usd`, `fee_revenue_24h_usd`) — tracked under existing backlog task `0125`. Will reuse the queue/worker/shared-lib shipped here. Picked up after 0191 proves out.
- **Asset USD price** (`assets.usd_price`, `assets.usd_price_updated_at`): identified as a stellarchain.io/markets parity gap. Will need a separate enrichment kind once a price source is chosen and a periodic refresh model is in place. Not in MVP.
- **Accounts metadata enrichment** (`accounts.first_seen_ledger`, `last_seen_ledger`, `sequence_number`, `account_balances_current`): blocked on task 0048 (accounts module). Should be designed _into_ 0048 rather than retrofitted.
- **`wasm_interface_metadata` runtime fetch**: needs Stellar RPC transport + retention strategy for pre-RPC contracts; separate task.
- **NFT metadata external fetch** (URI-pointer pattern): only relevant if/when Soroban NFTs adopt URI-pointer metadata. Today metadata is inline JSONB; nothing to enrich.
- **ADR 0029 amendment**: spawn a follow-up DOCS task to update ADR 0029 with the runtime_enrichment umbrella + SQS type-1 model description.

Open design questions inside the task (not blocking kickoff):

- **Galaxy SQS produce timing** — emit inside the indexer's write transaction (atomic but couples indexer availability to SQS) vs emit after commit (eventually consistent, needs a janitor for missed rows). Decide during implementation.

## Implementation Notes

### Crates touched

- **NEW** `crates/enrichment-shared/` (lib, ~250 lines incl. tests):
  - `sep1/{client,dto,errors,mod}.rs` — moved 1:1 from `api::runtime_enrichment::sep1`; replaced `crate::cache::ttl_future_cache` with inline `moka::future::Cache::builder()` to break the api-crate dependency. `Sep1Currency.image` field added (was missing — task 0188 didn't need it).
  - `enrich/{error,icon,mod}.rs` — new. `enrich_asset_icon(pool, asset_id, fetcher) -> Result<(), EnrichError>` is the single source of truth invoked by both worker (per SQS msg) and the future backfill CLI.
- **NEW** `crates/enrichment-worker/` (binary, ~130 lines):
  - `main.rs` — `lambda_runtime::run` over `SqsEvent`, internally tagged `EnrichmentMessage` enum (one variant `Icon { asset_id }` today, compiler-checked add of `LpTvl` etc. later), Permanent-vs-Transient `RecordError` so parse-level fails ack and only DB/network fails redeliver.
- **MODIFIED** `crates/indexer/`:
  - `handler/enrichment_publish.rs` (NEW) — `Publisher` struct with required `ENRICHMENT_QUEUE_URL` env var. Post-commit `SELECT DISTINCT a.id FROM assets ... WHERE icon_url IS NULL AND ((code, issuer_strkey) IN UNNEST OR contract_id = ANY)`. Batches 10 msgs/req via `SendMessageBatch`, body built via `serde_json::json!`.
  - `handler/process.rs` — `process_ledger` now returns `Vec<ExtractedAsset>` so callers that care (Galaxy Lambda handler) can publish; backfill / bench callers ignore the value (`let _ = …` via `?`).
  - `handler/mod.rs` — `HandlerState.enrichment_publisher: Publisher` field; SQS publish call lives here (Lambda-specific orchestration), not in shared `process_ledger`.
  - `main.rs` — `Publisher::from_env(sqs_client)?` propagates a missing env var to Lambda init failure. Same hard-fail model as `DATABASE_URL` / `SECRET_ARN`.
- **MODIFIED** `crates/api/src/runtime_enrichment/sep1/mod.rs` — collapsed to a 1-line re-export shim over `enrichment_shared::sep1`. All `crate::runtime_enrichment::sep1::…` import sites elsewhere in the api crate (main, tests, network::handlers, assets::handlers) keep working unchanged.
- **MODIFIED** `crates/api/src/cache.rs` — removed unused `ttl_future_cache` + `FutureCache` re-export (sole consumer was sep1, now in shared crate).
- **MODIFIED** `crates/backfill-runner/`, `crates/backfill-bench/` — **zero functional change**; reverted after a refactor pass that briefly threaded a `Publisher` arg through `process_ledger`. SQS produce kept Lambda-only.

### Infra

- **MODIFIED** `infra/src/lib/stacks/compute-stack.ts` — added `EnrichmentQueue` + `EnrichmentDlq` (DLQ retention 14d, `maxReceiveCount=3`), CW alarm on DLQ depth (5×1-min datapoints), `EnrichmentWorkerFunction` `RustFunction`, `SqsEventSource` (batchSize=10, reportBatchItemFailures, maxBatchingWindow=5s), IAM grants (indexer SendMessage; worker ConsumeMessages + dbSecret read), `ENRICHMENT_QUEUE_URL` injected into indexer env.
- **MODIFIED** `infra/src/lib/types.ts` — three new `enrichmentWorkerLambda{Memory,Timeout,Concurrency}` fields.
- **MODIFIED** `infra/envs/{staging,production}.json` — defaults: 256 MB / 30 s / `ReservedConcurrency=2`.

### Tests

- `cargo test -p enrichment-shared` — 14 passing (5 `validate_host` + 4 `dto::tests` + 5 `find_icon`).
- `cargo test -p api --lib` — 100 passing (no regression after sep1 move + helper inline).
- `cargo test -p enrichment-worker` — no unit tests (Lambda glue; behaviour covered by `enrich::icon` tests).
- `npx nx run @rumblefish/soroban-block-explorer-aws-cdk:typecheck` — clean.
- `npx nx run @rumblefish/api-types:generate` — clean diff (no api surface change).

## Design Decisions

### From Plan

1. **`enrichment-shared` lib crate** — sep1 fetcher moved out of `api` so the worker Lambda + future backfill can depend on it without a cyclic api dep.
2. **SQS standard queue + DLQ** — at-least-once delivery is fine; the worker is idempotent on the value level (always fetch + always write). FIFO would have been overkill.
3. **`maxReceiveCount=3`** — three transient redeliveries before the DLQ alarm fires. Below this the noise exceeds the signal; above it the operator alert lags too long.
4. **Worker `ReservedConcurrency=2`** — polite to issuer servers and bounded against accidental RDS connection exhaustion. Steady-state producer rate (12 ledger files / min × ~2 newly-inserted assets each) drains comfortably.
5. **Indexer publishes only un-enriched ids** (post-commit `WHERE icon_url IS NULL` filter) — natural backpressure; once an asset is enriched it drops out of the query so we don't re-emit it on every ledger that touches it.
6. **`process_ledger` stays in shared `crates/indexer`** — no separate Galaxy crate, just a Lambda-only handler module that wraps it.

### Emerged

7. **Required env var, hard-fail Lambda init** — first cut had a `Publisher::Disabled` enum variant that silently no-op'd if `ENRICHMENT_QUEUE_URL` was missing. Karol pushed back: silent disable in prod is the worst failure mode. Iterated through (a) `Result + Option<Publisher>` (decouple publish from ingest) → (b) `Result + ?` (hard fail Lambda init). Settled on (b) — same pattern as `DATABASE_URL` / `SECRET_ARN` already in `main.rs`. Misconfig in prod would surface immediately via CW Init Errors instead of leaking silent stale icons.
8. **`process_ledger` returns `Vec<ExtractedAsset>` instead of taking a `Publisher` arg** — mid-implementation pass added the publisher arg to `process_ledger`, which forced `backfill-runner` and `backfill-bench` to take a `Publisher::Disabled` arg in their call sites. Karol flagged: backfill should not be modified at all. Refactored: `process_ledger` returns the extracted assets (Galaxy Lambda handler does the SQS publish; backfill ignores the return). Net diff vs HEAD on backfill files: zero.
9. **`SELECT DISTINCT a.id`** — SAC assets (`asset_type=2`) carry both classic identity (`code, issuer`) and a `contract_id`. The producer's `extracted` slice can include both representations, which would otherwise yield duplicate ids and an SQS batch-entry-id collision. Defensive `DISTINCT` is cheap insurance.
10. **`EnrichmentMessage` as serde-tagged enum** — initial `struct { kind: String, asset_id: Option<i32> }` shape needed manual `MissingId` validation per kind. Replaced with `#[serde(tag = "kind")] enum EnrichmentMessage { Icon { asset_id: i32 } }` so the compiler enforces match-arm coverage when `LpTvl` etc. land later, and unknown / malformed payloads collapse to a single permanent-fail path.
11. **Permanent-vs-Transient worker error split** — first cut returned every `RecordError` variant as `Err`, which meant malformed JSON / unknown kind burned the SQS retry budget × 3 before landing in the DLQ. Split into `RecordError::Permanent(String)` (acked + ERROR log) and `RecordError::Transient(EnrichError)` (BatchItemFailure → SQS retry). Permanent fails surface via the ERROR log filter; transient fails recover via SQS redelivery and only escalate to the DLQ on sustained outage.
12. **`'' empty-string sentinel` for permanent fetch failures** — avoids the worker re-fetching dead issuers on every duplicate message. Re-runs naturally short-circuit on `WHERE icon_url IS NULL`. A future operator-driven `--force-retry` (in the Future Work backfill CLI) clears the sentinel.
13. **URL length pre-check (1024 byte CHECK constraint)** — issuer-published URLs that exceed `assets.icon_url VARCHAR(1024)` would otherwise fail the UPDATE with a CHECK violation that bubbles up as a transient SQS retry. Cap the URL on the application side and write the sentinel instead.
14. **Removed `ttl_future_cache` + `FutureCache` re-export from `api::cache`** — sole consumer was the sep1 fetcher, now in `enrichment-shared` with its own inline moka wiring. `api::cache` doc comment updated to note re-introduction once a second future-cache caller materialises.
15. **Inline find_currency helper in `assets::handlers`** — initially collapsed `find_currency` into `extract_sep1_fields` via `Option::zip()`. The project linter / a teammate edit restored the two-helper pattern; per the editor-intent system reminder we kept that style and re-added the import.
16. **API runtime_enrichment shim left in place** — could have removed `crates/api/src/runtime_enrichment/sep1/mod.rs` entirely by switching every api-internal `use crate::runtime_enrichment::sep1::…` to `use enrichment_shared::sep1::…`. Kept the 1-line shim because it minimises blast radius for the move and is trivially deletable later.

## Issues Encountered

- **Task ID collisions** — sequential allocator hit `fix/0189_lp-positions-fk-violation` (origin) and then `chore(lore-0190): spawn parse_error coverage task` (develop). Renumbered through `0189 → 0190 → 0191`. Final draft committed as `0191`.
- **`sqlx::query!` macros need `DATABASE_URL` at build time** — switched the `enrich_asset_icon` SELECT/UPDATE to `sqlx::query` non-macro with `.bind(...)` so the build is hermetic. No prepared-statement caching loss in practice (sqlx caches at runtime).
- **Type inference glitch on `.filter()` after `Option<String>::and_then`** — annotated `|s: &String|` to disambiguate.
- **Premature `Publisher` arg on `process_ledger`** — refactored out (see Design Decision 8). Backfill files ended up with zero diff vs HEAD.
- **Linter restored a removed helper** — `find_currency` in `assets::handlers` (see Design Decision 15). Took the linter's intent at face value rather than re-removing.
- **No `cdk diff` in this session** — no AWS creds available; operator runs at deploy. Captured under Acceptance Criteria as deferred.

## Future Work

- **Local backfill CLI subcommand** — `backfill-runner enrichment run /
status`. Reuses `enrichment_shared::enrich_and_persist::icon::enrich_asset_icon`
  to drain rows that pre-date the live producer. Streaming
  per-asset SELECT + `buffer_unordered(10)`, sentinel resume on
  `WHERE icon_url IS NULL`. Spawn a separate task when 0191 is in
  production and the live path is proven.
- **Add LP analytics kind** — extend `enrichment-shared::enrich` with
  `enrich_pool_tvl`, add `lp_tvl` arm to the worker dispatcher, add
  the columns migration and indexer SQS produce hook for new LP rows.
  Driven by task 0125.
- **Periodic refresh janitor**: cron-driven Lambda that scans
  `WHERE updated_at < NOW() - INTERVAL '30 days'` per kind and
  re-emits SQS messages for stale rows. Only needed once we observe
  stale icons in production. Spawn a follow-up task when symptoms
  appear.
- **Worker observability**: per-kind CloudWatch metrics (success/fail
  counters, fetch latency histogram). Initially we'll piggy-back on
  Lambda's standard metrics and the structured tracing logs.

## Notes

- **Branching**: cut from `develop` _after_ PR #157 (the 0188 SEP-1
  type-2 fetcher) merged. No code dependency between 0188 and 0191
  once 0188 landed. Branch:
  `feat/0191_type1-enrichment-worker-lambda`.
- **Implementation status**: shared crate, worker Lambda binary,
  indexer SQS producer, and CDK queue/worker/alarm wiring landed in
  a single squashed commit (`25c93c9`). Detailed module-level shape
  is captured under the "Implementation Notes", "Tests", "Design
  Decisions", and "Issues Encountered" sections above. Two operator-
  driven acceptance items remain — `cdk diff` clean on synth and
  DLQ poison-message verification in non-prod — see Acceptance
  Criteria.
