---
id: '0145'
title: 'Backfill runner: public Stellar S3 → parsed JSON.zst on our S3'
type: FEATURE
status: active
related_adr: ['0012', '0027', '0028']
related_tasks: ['0146', '0147', '0117', '0140', '0141', '0142']
blocked_by: ['0146']
tags:
  [
    layer-backend,
    layer-infra,
    priority-high,
    effort-large,
    backfill,
    adr-0012,
    adr-0027,
    adr-0028,
    onboarding,
  ]
milestone: 1
links:
  - lore/2-adrs/0012_lightweight-bridge-db-schema-revision.md
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
  - lore/2-adrs/0028_parsed-ledger-artifact-v1-shape.md
history:
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Created to pre-produce ADR 0012's `parsed_ledger_{seq}.json.zst` artifacts
      on our S3 bucket before schema migration (0142) lands. Reuses parser from
      0117 (archived backfill benchmark) but replaces the persist step with S3
      upload — schema-free, so this work is not blocked by 0142 / 0141.
  - date: '2026-04-20'
    status: backlog
    who: stkrolikiewicz
    note: >
      Scope narrowed: artifact shape + builder + serialization + key layout
      extracted to task 0146 (shared core). This task now covers only the
      runner — range planning, worker queue, fetch from public Stellar S3,
      idempotent upload, resume. Shape suitable as an onboarding task —
      self-contained, no schema decisions required. Parallel with live
      lambda (0147) once 0146 API is frozen.
  - date: '2026-04-21'
    status: active
    who: karolkow
    note: >
      Activated. Picking up the runner work.
---

# Backfill runner: public Stellar S3 → parsed JSON.zst on our S3

## Summary

Build a Rust CLI that drives a parallel backfill of the Soroban-era
Stellar pubnet archive. For each ledger: fetch `.xdr.zst` from the public
archive bucket, hand the batch to the shared artifact core (task 0146),
upload the resulting `parsed_ledger_{seq}.json.zst` to our S3 bucket
idempotently. No DB persist. No parser logic — this task consumes the
shared core, never reimplements it.

Shaped as an onboarding-friendly task: self-contained, real user value,
touches the interesting corners of the codebase (parser reuse, S3, Tokio
concurrency, workspace layout) without requiring decisions on the schema
refactor landing in parallel.

## Status: Backlog

**Current state:** not started. Blocked on task 0146 — shared artifact
core's public API must be frozen before the runner can target a stable
contract.

## Context

ADR 0012's S3-offload principle specifies one
`parsed_ledger_{sequence}.json.zst` per ledger in our S3 bucket; the
concrete artifact shape is defined by task 0146 (and formalized in
ADR 0028). Task 0117 (archived) benchmarked streaming
ledgers from the public Stellar S3 bucket through the existing Rust
parser. That benchmark wrote to PostgreSQL; this task replaces the
persist step with an S3 artifact upload.

Task 0146 owns the parse → JSON → zstd → S3 key pipeline as reusable
functions. This task wires source (public archive) + sink (our S3) +
concurrency + resume around those functions. Task 0147 does the live
counterpart for Galexie events. All three live downstream of fmazur's
ADR 0027 schema refactor (task 0140, branch
`refactor/lore-0140-adr-0027-schema`, commit `89f4335`), which rebuilt
the DB schema to ADR 0027 and inherits ADR 0012's S3-offload principle.

Doing backfill before the indexer DB schema (0142) lands means when the
indexer flips on, the artifact corpus already exists historically — DB
population becomes a walk over S3, not a re-parse from raw XDR.

## Scope

### In scope

1. **New CLI crate** — `crates/s3-backfill-pipeline/` Rust binary.
   Dependencies: `xdr-parser` (shared core from 0146), `aws-sdk-s3`,
   `tokio`, `clap`, `anyhow` (binary only — not in library surface),
   `tracing`.
2. **CLI subcommands** (v1):
   - `run --start <seq> --end <seq> [--workers N]` — the primary
     workflow.
   - `status [--range <start>-<end>]` — reports completed / missing
     ledgers by `HeadObject` against our bucket. No separate state store.
     `rehash-s3` or `verify` come later if needed; not in v1.
3. **Start ledger** — first Soroban-era ledger. Source of truth:
   `crates/backfill-bench/README.md` (ledger `50_457_424`, 2024-02-20).
   Documented there from community reference; treat as the working
   value. If SDF publishes an authoritative number, update in a
   follow-up, don't block v1. Fix the discrepancy with the 0145 original
   spec (`~53_795_300`) by deferring to `backfill-bench` README.
4. **Source fetch** — `aws-sdk-s3` with unsigned requests against
   `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`. Stream
   objects; decompression lives in shared core (0146 re-exports or
   `xdr-parser::decompress_zstd` directly — both are fine, pick one).
5. **Parse + artifact** — every ledger goes through
   `xdr_parser::artifact::build_parsed_ledger_artifact` →
   `serialize_artifact_json` → `compress_artifact_zstd`. Key from
   `parsed_ledger_s3_key`. **Do not reimplement** any of these.
6. **Idempotent upload** — `HeadObject` before `PutObject`; skip on
   match. Mirror the same policy as the live lambda (task 0147) so the
   two paths behave identically.
7. **Worker model** — `tokio::task::JoinSet` with a bounded channel.
   Range planner emits chunks (default 100 ledgers per job, tunable),
   workers pull jobs, each worker processes ledgers within its chunk
   sequentially. Concurrency cap = worker count; default a conservative
   value (4–8) — tune in README after a measurement run.
8. **Resume** — on startup, enumerate target prefix via `ListObjectsV2`,
   skip sequences already present. Optional local watermark file for
   fast warm-start; falls back to S3 listing on cold start. No separate
   DynamoDB / SQS — would be operational overhead disproportionate to
   this scope.
9. **Retry** — per-ledger retry with exponential backoff (3 attempts
   default) around fetch and upload. Parse errors are not retried —
   they indicate a data-shape bug and should surface immediately.
10. **Observability** — `tracing` logs per ledger (sequence, parse
    duration, JSON size, zstd ratio, upload duration). Periodic progress
    summary every N ledgers (configurable) with throughput + ETA. Exit
    code non-zero on unrecoverable failure. No CloudWatch metrics —
    this is an operator-run CLI, not a deployed service.
11. **README** — setup instructions, worker count guidance, measured
    throughput on a reference machine, cost notes.

### Out of scope

- **Shared artifact logic** — lives in 0146. Zero duplication here.
- **Live Galexie ingestion** — lives in 0147. Backfill and live path
  share only the artifact core; otherwise independent.
- **Any DB persist** — deferred.
- **Lambda / Fargate / ECS deployment** — this is an operator CLI run
  from a workstation or single EC2 instance. Throughput target does not
  justify a managed service; ledger parse is embarrassingly parallel
  and bounded primarily by CPU.
- **Pre-Soroban ledgers** — scope starts at Protocol 20 go-live.
- **Re-emit on schema change** — doable via `run --start … --end …`
  against an already-populated range. No separate mechanism.
- **Cross-region reads** — run the CLI in `us-east-1` (same region as
  the public archive) to keep ingress free.

## Implementation Plan

### Step 1 — Scaffold crate

`crates/s3-backfill-pipeline/` binary crate. Add to workspace. Pull in
dependencies. `cargo check` + Nx targets register.

### Step 2 — CLI skeleton

`clap` derive-style CLI with `run` and `status` subcommands. Config via
CLI flags + env for bucket names. No config file.

### Step 3 — Source fetcher

`aws-sdk-s3` unsigned client for `aws-public-blockchain`. Enumerate
partition prefixes → objects → by-sequence lookup.

### Step 4 — Range planner + worker pool

Range planner yields `Chunk { start, end }`. Bounded `mpsc` channel
sized to `workers * 2`. Each worker: pull chunk → loop sequences →
fetch → shared core → upload.

### Step 5 — Idempotent upload

`HeadObject` → conditional `PutObject`. Content-Type `application/zstd`.
No Content-Encoding.

### Step 6 — Resume

Cold start: `ListObjectsV2` over target prefix to build a bloom-filter
or `HashSet<u32>` of completed sequences. Warm start: local watermark
file. Skip completed in range planner.

### Step 7 — Observability + README

Structured logs, progress summary task, throughput measurement. README
documents reference throughput + recommended worker count.

### Step 8 — Staging dry-run

1000-ledger window against staging bucket. Verify idempotent re-runs
are no-ops. Verify status reports match.

### Step 9 — Production run

Full Soroban-era range → production bucket. Monitor progress, adjust
concurrency.

## Acceptance Criteria

- [ ] `crates/s3-backfill-pipeline/` builds and passes `nx run rust:build`,
      `nx run rust:test`, `nx run rust:lint`.
- [ ] Reads from `aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`
      unsigned.
- [ ] Emits artifacts byte-identical to those produced by task 0147 for
      the same ledger sequence (both routes go through shared core).
- [ ] `run` is idempotent: re-running over a completed range produces no
      uploads and no failures.
- [ ] `status` accurately reports completed / missing ledgers in a range.
- [ ] Resumes cleanly after SIGTERM — no duplicate uploads, no missed
      ledgers.
- [ ] Configurable worker count; throughput measured and documented in
      `crates/s3-backfill-pipeline/README.md`.
- [ ] Full Soroban-era range processed to production S3.
- [ ] No duplication of parser, serialization, or key-layout logic from
      0146 — every artifact operation calls into the shared core.

## Onboarding Notes

- **Fixed contract:** treat task 0146's public API as a contract. Do
  not modify `xdr-parser::artifact::*`. If you find a shape or behavior
  issue, raise it — don't patch it locally.
- **Reference crate:** `crates/backfill-bench/` has working code for
  streaming the public archive, partition math, and range iteration.
  Lift the patterns, not the DB persist path.
- **Nx commands:** `pnpm nx build rust`, `pnpm nx test rust`,
  `pnpm nx lint rust`. Avoid global `cargo` — use the workspace's
  package manager.
- **Branching:** cut from `refactor/lore-0140-adr-0027-schema` (not
  develop). Merge target is also that branch until the refactor
  finishes.
- **Ask early:** the first PR should land a scaffolded crate +
  fetch + one end-to-end ledger to production-like staging. Review
  gates early catch direction issues cheaply.

## Risks / Notes

- **Compute cost** — parsing tens of millions of ledgers is CPU-heavy.
  Benchmark from 0117 informs machine sizing. A single large EC2
  instance or a workstation should suffice; ECS/Fargate is not
  justified for a one-shot historical backfill.
- **Ingress cost** — zero inside `us-east-1`. Run there.
- **Egress from our S3 later** — indexer DB ingester (future task)
  should also run in `us-east-1` to keep that free.
- **Start ledger ambiguity** — `50_457_424` is community-sourced.
  Cross-verify with SDF opportunistically; a small leading gap is
  re-runnable cheaply.
- **Shape stability** — if 0146's API changes post-freeze, this task
  rebases, not redesigns. Keep coupling at the function-call layer.
