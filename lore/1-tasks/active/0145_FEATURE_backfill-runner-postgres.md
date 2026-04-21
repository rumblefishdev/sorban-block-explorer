---
id: '0145'
title: 'Backfill runner: public Stellar S3 → Postgres (ADR 0027)'
type: FEATURE
status: active
related_adr: ['0027']
related_tasks: ['0140', '0141', '0142', '0149']
blocked_by: []
tags:
  [
    layer-backend,
    layer-infra,
    priority-high,
    effort-large,
    backfill,
    adr-0027,
    onboarding,
  ]
milestone: 1
links:
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
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
  - date: '2026-04-21'
    status: active
    who: karolkow
    note: >
      Scope pivot: sink changes from S3 `parsed_ledger_{seq}.json.zst` artifact
      to Postgres via the shared parse-and-persist function
      (`indexer::handler::process::process_ledger`). Drops dependency on task
      0146 (artifact core) and the parallel-with-0147 constraint — runner and
      live path no longer share a sink. Task 0146 stays untouched; 0147 will be
      re-evaluated separately (not in scope here). Positioned as a
      production-grade evolution of `crates/backfill-bench`: same sink, real
      concurrency, native AWS SDK, retry/resume, subcommands. Crate renamed
      `s3-backfill-pipeline` → `backfill-runner`; task slug renamed
      `s3-backfill-pipeline-parsed-zstd` → `backfill-runner-postgres`.
---

# Backfill runner: public Stellar S3 → Postgres (ADR 0027)

## Summary

Build a Rust CLI that drives a parallel backfill of the Soroban-era
Stellar pubnet archive. For each ledger: fetch `.xdr.zst` from the public
archive bucket, parse it, and persist to Postgres via the shared
parse-and-persist function from the `indexer` crate. No S3 artifact. No
parser logic, no write-path logic — this task consumes the existing
`indexer::handler::process::process_ledger` entry point, never
reimplements it.

Framed as a production-grade evolution of `crates/backfill-bench`:
same sink (Postgres, ADR 0027 schema, via parse-and-persist), but with
real worker concurrency, native `aws-sdk-s3` (no `aws` CLI subprocess),
retry with backoff, `run` / `status` subcommands, and resume semantics
tuned for operator-driven historical runs.

## Status: Active

**Current state:** Unblocked — the parse-and-persist function is already exposed
via `indexer/src/lib.rs` and used by `backfill-bench` today. Task 0149
continues to evolve the write-path internals against ADR 0027; this
runner rebases as 0149 lands, no hard block.

## Context

Task 0140 (landed on master) rebuilt the DB schema to ADR 0027. Task
0149 wires the indexer write-path to that schema (`persist_ledger` body)
— in-flight on the current branch. `crates/backfill-bench/` is the
existing bench-quality runner: it calls the same
`indexer::handler::process::process_ledger` function, but shells out to
`aws s3 sync`, has no real concurrency inside a partition, no retry, no
subcommands, and carries dev shortcuts (local `DEFAULT` partition
bootstrap, sequential indexing, `.temp/` scratch dir).

This task builds the **production-grade** runner side-by-side with the
bench. Both sinks are the same Postgres via the same parse-and-persist
function — zero duplication of write-path logic. The difference is
operational: native AWS SDK, concurrent workers, retry, resume, and
observability suited to multi-day operator runs over tens of millions of
ledgers.

The earlier version of 0145 targeted a parallel S3 artifact corpus
(`parsed_ledger_{seq}.json.zst`) via a shared artifact core (task 0146).
That corpus is not needed for DB population — parse-and-persist writes
straight from XDR to normalized rows. The artifact path (0146) stays
intact in the repo but is not a dependency here; the live-path
counterpart (0147) is being re-evaluated separately.

## Scope

### In scope

1. **New CLI crate** — `crates/backfill-runner/` Rust binary.
   Dependencies: `indexer` (for `handler::process::process_ledger`),
   `xdr-parser` (for `decompress_zstd` + `deserialize_batch`),
   `db` (for pool creation), `aws-sdk-s3`, `tokio`, `clap`,
   `tracing`, `chrono`, `sqlx` (workspace).
2. **CLI subcommands** (v1):
   - `run --start <seq> --end <seq> [--workers N]` — the primary
     workflow.
   - `status [--range <start>-<end>]` — reports ingested / missing
     ledgers by querying the `ledgers` table for that range. No
     separate state store.
3. **Start ledger** — first Soroban-era ledger. Source of truth:
   `crates/backfill-bench/README.md` (ledger `50_457_424`, 2024-02-20).
   If SDF publishes an authoritative number, update in a follow-up —
   don't block v1.
4. **Source fetch** — `aws-sdk-s3` with unsigned requests against
   `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`. Stream
   objects directly into memory — **no local scratch dir**, no `aws`
   CLI subprocess. Decompression via `xdr_parser::decompress_zstd`.
5. **Parse + persist** — every ledger goes through
   `xdr_parser::deserialize_batch` →
   `indexer::handler::process::process_ledger`. **Do not reimplement**
   any write-path logic. The function is the contract.
6. **Idempotency** — `process_ledger` already handles the existence
   check in its own pathway; in addition, the range planner skips
   sequences already present in `ledgers` before enqueueing work, so
   workers don't even fetch blobs for completed ledgers.
7. **Worker model** — `tokio::task::JoinSet` with a bounded channel.
   Range planner emits chunks (default 100 ledgers per job, tunable),
   workers pull jobs, each worker processes ledgers within its chunk
   sequentially. Concurrency cap = worker count; default a conservative
   value (4–8) — tune in README after a measurement run. Per-worker
   Postgres connection via the shared `db::pool`.
8. **Resume** — on startup, query the `ledgers` table for existing
   sequences in the requested range (single scan), build a
   `HashSet<u32>`, and skip those in the range planner. Optional local
   watermark file for fast warm-start. No separate state store.
9. **Retry** — per-ledger retry with exponential backoff (3 attempts
   default) around S3 fetch. Parse errors are not retried — they
   indicate a data-shape bug and should surface immediately. Persist
   errors surface immediately too: schema / constraint violations are
   bugs, not transient failures.
10. **Observability** — `tracing` logs per ledger (sequence, parse
    duration, persist duration, file size). Periodic progress summary
    every N ledgers (configurable) with throughput + ETA. Exit code
    non-zero on unrecoverable failure. No CloudWatch metrics —
    operator-run CLI, not a deployed service.
11. **README** — setup instructions, worker count guidance, measured
    throughput on a reference machine, cost notes, diff vs
    `backfill-bench` (why both exist).

### Out of scope

- **S3 artifact corpus** — task 0146 owns the artifact core. Not
  consumed here. Not emitted here. The corpus exists or doesn't exist
  in parallel; this runner does not care.
- **Live Galexie ingestion** — a separate concern. Backfill and any
  future live path share only the parse-and-persist function.
- **Lambda / Fargate / ECS deployment** — operator CLI run from a
  workstation or single EC2 instance. Throughput target does not
  justify a managed service; ledger parse is embarrassingly parallel
  and bounded primarily by CPU.
- **Replacing `backfill-bench`** — the bench stays as the reference /
  benchmark prototype. This runner is the production tool; coexistence
  is intentional.
- **Pre-Soroban ledgers** — scope starts at Protocol 20 go-live.
- **`DEFAULT` partition bootstrap** — unlike `backfill-bench`, the
  production runner assumes the partition-management Lambda (or the
  completion of 0149's partition-provisioning work) handles partition
  ranges authoritatively. Not the runner's job to paper over.
- **Re-persist on schema change** — the write-path contract changing
  is 0149's problem; a re-run over a populated range is the fix.
- **Cross-region reads** — run the CLI in `us-east-1` (same region as
  the public archive) to keep ingress free.

## Implementation Plan

### Step 1 — Scaffold crate

`crates/backfill-runner/` binary crate. Add to workspace `members`.
Pull in dependencies (`indexer`, `xdr-parser`, `db`, `aws-sdk-s3`,
`aws-config`, `tokio`, `clap`, `tracing`, `chrono`, `sqlx`).
`cargo check` + Nx targets register.

### Step 2 — CLI skeleton

`clap` derive-style CLI with `run` and `status` subcommands. Config
via CLI flags + env for `DATABASE_URL`. No config file.

### Step 3 — Source fetcher

`aws-sdk-s3` unsigned client for `aws-public-blockchain`. Partition
math lifted from `backfill-bench` (`partitions_for_range`, hex
prefix). Stream each `.xdr.zst` object to a `Vec<u8>` via
`GetObject` — no disk writes.

### Step 4 — Range planner + worker pool

Build the completed-sequences set (one `SELECT sequence FROM ledgers
WHERE sequence BETWEEN $1 AND $2`). Range planner yields
`Chunk { start, end }` skipping completed. Bounded `mpsc` channel
sized to `workers * 2`. Each worker: pull chunk → loop sequences →
fetch → decompress → deserialize_batch → `process_ledger` per
`LedgerCloseMeta`.

### Step 5 — Retry

`tokio_retry` or hand-rolled exponential backoff around fetch.
No retry on parse / persist.

### Step 6 — Resume + watermark

Cold start: sequence-set query. Warm start: local watermark file
(highest contiguous sequence). Planner consults both. No race —
single writer.

### Step 7 — Observability + README

Structured logs, progress summary task, throughput measurement.
README documents reference throughput, recommended worker count,
and diff vs `backfill-bench`.

### Step 8 — Staging dry-run

1000-ledger window against staging DB. Verify idempotent re-runs
are no-ops. Verify `status` matches row counts.

### Step 9 — Production run

Full Soroban-era range → production DB. Monitor progress, adjust
concurrency.

## Acceptance Criteria

- [ ] `crates/backfill-runner/` builds and passes `nx run rust:build`,
      `nx run rust:test`, `nx run rust:lint`.
- [ ] Reads from `aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`
      unsigned, via `aws-sdk-s3` — no `aws` CLI subprocess.
- [ ] Persists via `indexer::handler::process::process_ledger`. No
      reimplementation of write-path logic.
- [ ] `run` is idempotent: re-running over a completed range produces
      no writes and no failures (existence check in planner + in
      persist function).
- [ ] `status` accurately reports ingested / missing ledgers in a
      range (row count in `ledgers` table).
- [ ] Resumes cleanly after SIGTERM — no duplicate rows, no missed
      ledgers.
- [ ] Configurable worker count; throughput measured and documented in
      `crates/backfill-runner/README.md`.
- [ ] README includes a diff vs `crates/backfill-bench` (why both
      exist).
- [ ] Full Soroban-era range processed to production DB.

## Onboarding Notes

- **Fixed contract:** treat `indexer::handler::process::process_ledger`
  as a contract. Do not modify the write-path from this task. If you
  find a shape or behavior issue, raise it — 0149 owns the write-path
  body.
- **Reference crate:** `crates/backfill-bench/` is the prototype.
  Lift patterns (partition math, hex layout, range iteration). Do not
  lift: the `aws s3 sync` subprocess, sequential-per-partition
  indexing, local `DEFAULT` partition bootstrap, `.temp/` scratch dir.
- **Nx commands:** `pnpm nx build rust`, `pnpm nx test rust`,
  `pnpm nx lint rust`. Avoid global `cargo` — use the workspace's
  package manager.
- **Ask early:** the first PR should land a scaffolded crate +
  fetch + one end-to-end ledger persisted to a staging DB. Review
  gates early catch direction issues cheaply.

## Risks / Notes

- **Compute cost** — parsing tens of millions of ledgers is CPU-heavy.
  A single large EC2 instance or a workstation should suffice;
  ECS/Fargate is not justified for a one-shot historical backfill.
- **DB write throughput** — the bottleneck may shift from parse to
  persist under high worker counts. Measure. If persist-bound, cap
  workers at DB's sustainable concurrent-write level rather than
  CPU-count.
- **Ingress cost** — zero inside `us-east-1`. Run there.
- **Start ledger ambiguity** — `50_457_424` is community-sourced.
  Cross-verify with SDF opportunistically; a small leading gap is
  re-runnable cheaply.
- **Write-path stability** — if 0149 changes the `process_ledger`
  signature or semantics, this task rebases. Keep coupling at the
  single function-call layer.
- **Partition coverage** — the runner assumes partitioned tables have
  ranges provisioned for the backfill window. If 0149 / the
  partition-management Lambda doesn't cover the historical range at
  run time, the runner fails fast rather than auto-provisioning.
