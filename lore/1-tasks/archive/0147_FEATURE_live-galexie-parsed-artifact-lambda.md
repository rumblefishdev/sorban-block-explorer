---
id: '0147'
title: 'Live Galexie onPut lambda: raw XDR → parsed ledger JSON.zst on S3'
type: FEATURE
status: superseded
related_adr: ['0012', '0027', '0028', '0029']
related_tasks: ['0146', '0145', '0044', '0029', '0149', '0033']
blocked_by: []
tags:
  [
    layer-infra,
    layer-indexer,
    priority-high,
    effort-medium,
    adr-0012,
    adr-0027,
    adr-0028,
    lambda,
    live-path,
  ]
milestone: 1
links:
  - lore/2-adrs/0012_lightweight-bridge-db-schema-revision.md
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
  - lore/2-adrs/0028_parsed-ledger-artifact-v1-shape.md
history:
  - date: '2026-04-20'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned alongside 0145/0146. Live-path counterpart to backfill (0145),
      both consuming shared artifact core (0146). Emits the same parsed
      JSON.zst artifacts as backfill so the corpus on S3 is single-format,
      single-source-of-truth, regardless of which path produced a given
      ledger.
  - date: '2026-04-21'
    status: superseded
    who: stkrolikiewicz
    by: ['0149']
    note: >
      Archived as superseded by ADR 0029 architectural pivot. The existing
      indexer lambda (task 0033, deployed) already subscribes to the Galexie
      bucket's onPut and calls `indexer::handler::process::process_ledger` →
      `persist_ledger`. Once task 0149 fills `persist_ledger` against the
      ADR 0027 schema, the existing lambda writes to DB directly — no
      artifact emission, no separate "artifact-producer" lambda required.
      The motivating artifact-emission scope of this task no longer exists;
      any remaining infra tweaks (DLQ, concurrency, alarms) are trivial
      chore work, not a standalone FEATURE task.
---

# Live Galexie onPut lambda — raw XDR → parsed ledger JSON.zst

## Summary

Build a new AWS Lambda, triggered by S3 `ObjectCreated` on the Galexie raw
`.xdr.zst` bucket, that parses each incoming ledger and writes a
`parsed_ledger_{seq}.json.zst` to our parsed-ledger S3 bucket. Uses the
shared artifact core from task 0146. No DB persist. This is the live path
to the same corpus that task 0145 populates historically.

## Status: Backlog

**Current state:** not started. Blocked on 0146 (public API freeze required
before this lambda can compile against the shared core). Base branch:
`refactor/lore-0140-adr-0027-schema` — cut after 0146 lands there.

## Context

Galexie exports raw `.xdr.zst` ledger batches onto our ingestion bucket.
Today a single indexer lambda reads those, parses them, and writes to
PostgreSQL in one pass — mixing parse and persist responsibilities
(`crates/indexer/src/handler/process.rs`). Under the ADR 0027 schema
(fmazur's task 0140 refactor, commit `89f4335`, inherits ADR 0012's
S3-offload principle), heavy payloads move out of the DB and live on S3
as ADR 0012 artifacts.

This task introduces the first half of that split: a dedicated
**artifact-producer** lambda. A future task (not this one) replaces or
refits the existing indexer to consume produced artifacts and write only
the lightweight DB rows. Splitting now, before that follow-up, gives us:

- a clean single-responsibility lambda for parsing + artifact emission,
- a stable artifact corpus on S3 that the DB ingester can replay from,
- identical format to backfill (task 0145) since both call into shared
  core (task 0146).

## Scope

### In scope

1. **New Rust binary** — `crates/artifact-producer-lambda/` (or colocated
   under `crates/indexer/bin/` — decide during Step 1). One lambda, one
   purpose.
2. **Trigger** — S3 `ObjectCreated:*` event on the raw Galexie bucket,
   filtered by suffix `.xdr.zst` (mirrors existing indexer lambda config
   in `infra/src/lib/stacks/compute-stack.ts`).
3. **Event handling** — for each `Records[]` entry:
   1. `parse_s3_key` (already in `xdr-parser`) → ledger sequence range.
   2. `GetObject` raw bytes from source bucket.
   3. `decompress_zstd` → XDR bytes (existing `xdr-parser` API).
   4. `deserialize_batch` → `LedgerCloseMetaBatch` (existing).
   5. For each `LedgerCloseMeta` in batch: `build_parsed_ledger_artifact`
      (from 0146) → `serialize_artifact_json` → `compress_artifact_zstd`.
   6. Idempotent `PutObject` to parsed bucket at `parsed_ledger_s3_key(seq)`.
4. **Idempotency** — `HeadObject` before `PutObject`; skip if artifact
   exists with matching size. Alternative: `PutObject` with
   `If-None-Match: *` if the SDK version supports it cleanly. Pick one
   and document.
5. **Error handling** — batch-level failures return an error from the
   handler → Lambda retries per its retry config → dead-letter queue on
   final failure. Partial-batch emission **not** allowed: either the whole
   event processes or the whole event fails. This avoids orphaned
   artifacts that silently mask an unprocessed ledger.
6. **Configuration** — env vars only:
   - `PARSED_LEDGER_BUCKET` — target bucket for artifacts.
   - `RUST_LOG` — tracing level.
     No Parameter Store, no Secrets Manager. Lambda role grants
     `s3:GetObject` on source, `s3:HeadObject` + `s3:PutObject` on target.
7. **Observability** — tracing logs per ledger: sequence, parse duration,
   JSON size, zstd ratio, upload duration. CloudWatch default metrics
   only (no custom metrics in v1 — lambda duration + errors + invocations
   are enough to detect pipeline health). Add custom metrics only when a
   specific question can't be answered from logs.
8. **CDK wiring** — extend `infra/src/lib/stacks/compute-stack.ts`
   (existing pattern): new `NodejsFunction` or Rust lambda construct,
   S3 event trigger, DLQ reused from existing indexer stack if suitable,
   otherwise new DLQ with identical config. Concurrency cap configurable
   (follow pattern from task 0033 / commit `e8f900c`).

### Out of scope

- **Any DB persist** — this lambda writes only to S3. DB ingester is a
  separate follow-up.
- **Refitting the existing indexer lambda** — existing path untouched.
  Both lambdas subscribe to the same bucket during transition; the DB
  ingester follow-up will eventually replace `handler/process.rs`.
  Double-subscription during transition is intentional and cheap.
- **Backfill of historical data** — task 0145.
- **Retroactive re-parse** — if artifact shape changes, re-emit is the
  backfill runner's job (task 0145 can re-run for any range).
- **Reading parsed artifacts** — consumers are a later task.
- **Cross-region replication of parsed bucket** — not needed for M1;
  same-region reads from indexer DB lambda cover the critical path.
- **Custom CloudWatch metrics / alarms** — stock lambda metrics first.
  Add custom only on demonstrated need.

## Implementation Plan

### Step 1 — Decide binary layout

Colocate under `crates/indexer/` as a second binary target, or separate
crate. Lean toward colocation to share Cargo deps and keep the parser
wiring in one place; separate only if deploy-size forces it.

### Step 2 — Handler skeleton

`lambda_runtime` + `aws_lambda_events::s3::S3Event`. Pattern-match records,
call `parse_s3_key`, delegate per-ledger to shared core.

### Step 3 — S3 client wiring

`aws_sdk_s3::Client` via default provider chain (lambda exec role
credentials). Two buckets: source read-only, target write + head.

### Step 4 — Idempotent upload

`HeadObject` → on 404: `PutObject`; on 200 with matching size: skip.
Other errors: bubble up.

### Step 5 — CDK

Add lambda construct to compute stack. Wire S3 event with
`suffix: '.xdr.zst'`. DLQ + retry config mirror existing indexer lambda
settings (see fix commit `e8f900c`).

### Step 6 — Integration test against staging

Deploy to staging stack. Drop a known Galexie `.xdr.zst` into source
bucket. Assert artifact lands at expected key with expected content
(decompress + decode + field-compare against golden from 0146 fixtures).

### Step 7 — Production rollout

Enable in prod with low concurrency cap first (e.g. 2). Monitor lambda
errors + duration for 24h. Ramp concurrency once steady.

## Acceptance Criteria

- [ ] Lambda triggered by S3 `ObjectCreated` with `.xdr.zst` suffix on
      Galexie source bucket.
- [ ] Emits artifact per ledger at `parsed_ledger_s3_key(seq)` using
      shared core from 0146.
- [ ] Idempotent: re-delivery of the same event does not duplicate work
      and does not fail.
- [ ] Batch-level atomicity: partial-batch failures return error and are
      retried; no orphaned artifacts.
- [ ] DLQ configured; messages land there after final retry.
- [ ] Lambda IAM role scoped to the two buckets only — no wildcard.
- [ ] Staging smoke test: ingested ledger produces artifact whose decoded
      content matches an equivalent 0146 golden.
- [ ] Artifacts emitted by this lambda and by backfill (0145) are
      byte-identical for the same ledger sequence.
- [ ] CDK diff + `nx run rust:build`, `nx run rust:test`, `nx run rust:lint`
      all pass.

## Risks / Notes

- **Double subscription during transition** — both this lambda and the
  existing indexer lambda react to the same `.xdr.zst` events. Cost is
  one extra lambda invocation per ledger until the DB ingester follow-up
  replaces the existing indexer. Acceptable; measured in cents/day at
  our volume.
- **Artifact shape drift vs live path** — prevented by shared core
  (0146). If the core's API is changed after freeze, this lambda and the
  backfill must update in lockstep.
- **Cold-start size** — Rust lambda cold starts are fast but binary size
  matters. Strip release build; avoid pulling `tokio` feature flags
  beyond what's needed.
- **Reserved concurrency** — do not copy indexer's production concurrency
  blindly. This lambda has different memory + duration profile (no DB
  round trips). Size independently after staging load test.
- **No Parameter Store** — env vars only. If secrets ever appear in
  config (they shouldn't for this scope), revisit.
- **S3 list consistency** — not relied on. Idempotency uses `HeadObject`
  on the exact key, which is strongly consistent.
