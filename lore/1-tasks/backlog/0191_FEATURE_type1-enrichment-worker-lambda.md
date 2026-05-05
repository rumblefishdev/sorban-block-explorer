---
id: '0191'
title: 'Type-1 enrichment live path (icon only): SQS-driven worker for assets.icon_url'
type: FEATURE
status: backlog
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

## Status: Backlog

**Current state:** Drafted. Open items listed under "Out of scope /
Open questions" — none block kicking the task off.

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
    └── enrich/
        ├── mod.rs
        └── icon.rs             ← pub async fn enrich_asset_icon(pool, asset_id) -> Result
```

The `enrich_asset_icon` function is the **single source of truth** for
"given this asset id, fetch its issuer's stellar.toml, extract the
matching `CURRENCIES[].image`, and write `icon_url`". Worker calls it
per SQS message; the future backfill CLI will call it per row from a
streaming SELECT; the live api never calls it (api uses the type-2
path for description / home_page only).

Behaviour:

1. `SELECT issuer, code, home_domain FROM assets WHERE id = $1`.
2. If `home_domain IS NULL` → write `''` empty sentinel + `Ok(())` (asset has no issuer-published toml; nothing to fetch). Future re-run won't refetch.
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
2. For each record, parse JSON message body: `{ kind: "icon", asset_id: u64 }`.
3. Match `kind`:
   - `"icon"` → `enrichment_shared::enrich::icon::enrich_asset_icon(&pool, asset_id).await`
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
{ "kind": "icon", "asset_id": 12345 }
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

- [ ] `crates/enrichment-shared` lib crate builds, hosts SEP-1 fetcher + `enrich_asset_icon`.
- [ ] `crates/api/src/runtime_enrichment/sep1/` re-exports from `enrichment-shared` (or is removed entirely if the api crate can import directly without breaking the OpenAPI/schema layout).
- [ ] `crates/enrichment-worker` Lambda binary builds, deployable, parses `SqsEvent`, dispatches `kind: "icon"` to `enrich_asset_icon`.
- [ ] Worker writes are unconditional overwrites — duplicate messages succeed and update `icon_url` to whatever the source currently returns.
- [ ] `crates/indexer` emits an SQS message for each newly inserted asset. Logged on a tracing span tagged with `kind` and `asset_id`.
- [ ] CDK provisions the queue, DLQ, alarm, worker Lambda, event source mapping, and IAM. `cdk diff` clean on a stack synth.
- [ ] DLQ alarm verified by injecting a poison message in a non-prod env (worker fails, message moves to DLQ, alarm fires).
- [ ] `0124` marked superseded and moved to archive in the same PR.
- [ ] **Docs updated** — `docs/architecture/database-schema/database-schema-overview.md` (or its successor enrichment topology section) describes the SQS-driven type-1 path. `assets.icon_url` already documented; no schema change. Per ADR 0032.
- [ ] **API types regenerated** — only required if `crates/api/**` is touched. Likely yes, because the runtime_enrichment re-export shuffle changes the api crate's deps. Run `npx nx run @rumblefish/api-types:generate` and commit `libs/api-types/src/{openapi.json,generated/}`.

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

## Future Work

- **Local backfill CLI subcommand** — `backfill-runner enrichment run /
status`. Reuses `enrichment_shared::enrich::icon::enrich_asset_icon`
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
  type-2 fetcher) merges. There is no code dependency between 0188 and
  0191 once 0188 lands. Branch name:
  `feat/0191_type1-enrichment-icon-worker`.
- **No commits / stages / pushes from the drafting session.** Per
  Karol's standing rule, the task md is written but not committed.
  Promotion to `active/` and any branch creation are explicit
  follow-up steps the operator drives.
