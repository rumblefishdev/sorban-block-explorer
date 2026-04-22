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
  - date: '2026-04-22'
    status: active
    who: karolkow
    note: >
      Design snapshot — consolidates all pivots and reversals from the
      implementation phase into the set of decisions that actually stand.
      Earlier incremental history (decision locks followed by several
      reversals) collapsed into this single final spec.

      **Sink pivot:** S3 `parsed_ledger_{seq}.json.zst` artifact → Postgres
      via `indexer::handler::process::process_ledger`. Drops dependency on
      task 0146 (artifact core) and the parallel-with-0147 constraint —
      runner and live path no longer share a sink. 0146 stays untouched;
      0147 is re-evaluated separately. Crate renamed `s3-backfill-pipeline`
      → `backfill-runner`; slug → `backfill-runner-postgres`.

      **Unit of work:** one 64k-ledger partition, synced via `aws s3 sync`
      subprocess into a local temp dir. Not per-ledger `GetObject` against
      `aws-sdk-s3` (ruled out — 64k ranged reads per partition is
      dramatically more expensive than one sync). No native SDK.

      **Concurrency:** single-threaded indexer, one ledger at a time, one
      partition at a time. The **only** background task is a single-slot
      prefetch of partition N+1 while N indexes — same pattern as
      `backfill-bench`. No worker pool, no `JoinSet`, no mpsc.

      **Resume (two DB-backed layers, no state file / marker / manifest):**
      (1) **Pre-sync partition skip** — at startup, after loading the
      `HashSet<u32>` of completed sequences, filter partitions down to
      `todo` via `partition_fully_done(p, start, end, &completed)` checking
      the clamped range `max(start, p.start)..=min(end, p.end)`. Required
      because cleanup-after-index removes local folders, so without this
      filter a re-run would re-download ~1–2 GB per already-done partition
      just for Stage B to reject every persist. (2) **Per-ledger skip
      inside a partition** — handles mid-partition crashes (pre-sync filter
      correctly lets a partial partition through, per-ledger filter skips
      the already-persisted prefix). Stage A (the sync itself) needs no
      marker — `aws s3 sync` is natively idempotent (LIST-only against a
      fully-synced dir), so every partition that survives the pre-sync
      filter gets unconditionally synced.

      **Cleanup-after-index (mandatory):** after a successful
      `index_partition(N)`, `tokio::fs::remove_dir_all` the local folder
      before awaiting the N+1 prefetch. Bounds disk at ~2 × partition_size
      regardless of range width. Cleanup failure is a hard error (`?`
      propagation) — silent warn-and-continue would accumulate the garbage
      cleanup was introduced to prevent. On _error_ paths cleanup is
      deliberately skipped (forensics + partial-sync preserved for next
      `aws s3 sync` to fill in).

      **Retry:** hand-rolled exponential backoff around `aws s3 sync` only
      — 3 attempts, 2s base, ×2 multiplier, 30s cap, hardcoded consts.
      Parse / persist are **not** retried; data-shape / schema bugs must
      surface immediately.

      **Debug-first error stance:** while 0149's write-path is still in
      flux, warn+continue is exactly the pattern to avoid. Post-sync
      missing file → `assert!(path.exists())`. Top-level `Result` in
      `main.rs` → `.expect(...)`. Invalid range (start > end) → `assert!`
      (no `BackfillError::InvalidRange` variant). Revisit once staging
      runs are stable.

      **Observability** (`run` = live `tail -f` stream; `status` = the
      structured "where am I" query — zero overlap):
      - Format: default human-readable `tracing` formatter (not JSON).
        `key=value` fields are enough for grep / awk; JSON was noise on
        `tail -f` and defended a use case `status` covers natively.
      - `--verbose` / `-v` is the only log-level dial. Without it filter
        is `warn` (quiet run, only retry warnings + panics); with it,
        per-ledger and per-partition `info` events flow.
      - Per-ledger event: `seq`, `partition`, `bytes`, `parse_ms`,
        `persist_ms`. Decompress deliberately **not timed** —
        deterministic zstd on fixed input, no diagnostic signal.
      - Per-partition boundary event: sync duration, file count, total
        bytes, parse/persist totals, **min/max per-ledger total_ms**
        (parse + persist, aggregated at run level via `combine_min` /
        `combine_max`), wall clock, throughput. No count-based
        intra-partition progress event — per-partition boundary at
        ~64k ledgers is the natural ~10-min checkpoint.
      - **Final run summary** → `println!`, not tracing. Always prints
        regardless of `--verbose` — one "what just happened" block:
        partitions processed, ledgers indexed / already-in-DB, parse
        total, persist total, ledger time min / max, elapsed.
      - `status` is `println!`-only too — no tracing; point-in-time CLI
        query, not a debug stream. Output includes per-partition `range /
        indexed / pending` table + `=== summary ===` block ("partitions
        fully indexed: X / Y", "ledgers indexed: X / Y (Z%)"). Local
        temp dir is not inspected — cleanup makes it a transient signal.

      **Preflight at startup:** `aws --version` + `SELECT 1`, both
      panic-on-fail. Operator / environment errors, not transient.

      **Partition helpers** (`Partition`, `partitions_for_range`,
      `s3_folder`, `local_folder`, `local_ledger_path`) **copied 1:1**
      from `backfill-bench`. No shared crate extraction yet — revisit
      if a third consumer appears.

      **Exit codes:** `0` on success; unrecoverable failure → panic
      (non-zero + stack trace). Signal-specific codes not differentiated.

      **SIGTERM / Ctrl-C:** abort in place; the two resume filters
      cover the restart. No graceful-shutdown plumbing.
---

# Backfill runner: public Stellar S3 → Postgres (ADR 0027)

## Summary

Build a Rust CLI that drives a **sequential, single-threaded** backfill of the
Soroban-era Stellar pubnet archive, partition by partition. For each
64k-ledger partition: `aws s3 sync` the partition into a local temp directory,
then iterate its `.xdr.zst` files in order — decompress, parse, persist via
the shared parse-and-persist function from the `indexer` crate. While one
partition indexes, the **next** partition syncs in the background, so the
indexer never blocks on S3 between partitions.

No parser logic, no write-path logic — this task consumes the existing
`indexer::handler::process::process_ledger` entry point, never reimplements it.

Framed as the **production-grade wrapper** around the same sync + index flow
that `crates/backfill-bench` prototypes. Same sink (Postgres, ADR 0027 schema,
via parse-and-persist), same unit of work (one partition), same partition
prefetch pattern. The difference from the bench is operational hardening:
`run` / `status` subcommands, structured logging with **per-stage timing**
(sync duration, decompress, parse, persist — logged for every ledger and
aggregated per partition), two-layer DB-based resume (per-partition
pre-sync skip + per-ledger skip), cleanup-after-index to bound disk
footprint, and a process shape suited for multi-day operator runs.

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
`indexer::handler::process::process_ledger` function, shells out to
`aws s3 sync` per partition into a local `.temp/` dir, and prefetches
the next partition while the current one indexes. It carries dev
shortcuts — local `DEFAULT` partition bootstrap, minimal logging, no
subcommands, no retry, no resume semantics — but its overall shape
(partition sync + sequential index + background prefetch) is the
**right** shape and is what this runner inherits.

This task builds the **production-grade** runner side-by-side with the
bench. Both sinks are the same Postgres via the same parse-and-persist
function — zero duplication of write-path logic. Both use the same
unit of work — one 64k-ledger partition synced via `aws s3 sync` into
a local temp dir. The delta is operator-facing: `run` / `status`
subcommands, per-stage timing instrumentation, two-layer DB-based
resume, cleanup-after-index, structured logs, defined exit codes,
and a README calibrated for a multi-day backfill rather than a bench
run.

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
   `db` (for pool creation), `tokio`, `clap`, `tracing`,
   `tracing-subscriber`, `chrono`, `sqlx` (workspace). No
   `aws-sdk-s3` — sync is delegated to the `aws` CLI subprocess
   (see point 4).
2. **CLI subcommands** (v1):
   - `run --start <seq> --end <seq>` — the primary workflow.
   - `status [--range <start>-<end>]` — reports ingested / missing
     ledgers by querying the `ledgers` table for that range. No
     separate state store.
3. **Start ledger** — first Soroban-era ledger. Source of truth:
   `crates/backfill-bench/README.md` (ledger `50_457_424`, 2024-02-20).
   If SDF publishes an authoritative number, update in a follow-up —
   don't block v1.
4. **Source fetch — partition sync via `aws s3 sync`** — the unit of
   transfer is a **whole 64k-ledger partition**, not individual
   ledgers. The public archive is laid out as partitioned directories
   under `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`; a
   single `aws s3 sync --no-sign-request` of one partition is
   dramatically cheaper than 64k ranged reads. Files land in a local
   temp directory (`--temp-dir` flag, default `.temp/backfill-runner/`).
   Subprocess management (stdout/stderr capture, exit status, duration
   logging) lives in the runner.
5. **Parse + persist** — for each ledger file in the partition, in
   sequence: `xdr_parser::decompress_zstd` →
   `xdr_parser::deserialize_batch` →
   `indexer::handler::process::process_ledger`. **Do not reimplement**
   any write-path logic. The function is the contract.
6. **Two-layer resume / idempotency** — the DB is the sole source
   of truth; there is no state file, no manifest, no marker.
   - **Pre-sync partition skip (coarse, runs first):** after
     building the `HashSet<u32>` of already-persisted sequences
     (see next bullet), filter the enumerated partition list down
     to the ones that are **not** fully done. A partition counts as
     fully done when every ledger in its _clamped_ range —
     `max(start, p.start)..=min(end, p.end)` — is in the set.
     Skipped partitions are neither synced nor indexed; with
     cleanup-after-index this is what prevents a re-run over a
     completed range from re-downloading ~1–2 GB per partition
     just to have Stage B reject every persist.
   - **Stage B — per-ledger persist skip (fine, still necessary):**
     inside a partition that the pre-sync filter lets through, skip
     `process_ledger` for any sequence already in the set. Required
     for mid-partition crashes, where a partition is only _partially_
     in DB and the pre-sync filter correctly lets it through.
   - **Stage A (sync itself):** no marker, no manifest, no file-count
     check. `aws s3 sync` is idempotent — a second call against a
     fully-synced dir is a cheap LIST with no GETs. For every
     partition that survives the pre-sync skip, the runner
     unconditionally invokes `aws s3 sync`; a partial dir from a
     crashed previous run is filled in, not corrupted.
7. **Sequential execution — single-threaded indexer** — the indexer
   runs one ledger at a time, one partition at a time. **No worker
   pool, no tokio `JoinSet` of indexer tasks, no bounded mpsc for
   ledger work.** Concurrency is explicitly out of scope for this
   runner. The one exception is the background prefetch task (next
   point).
8. **Background partition prefetch + mandatory cleanup** — while
   partition _N_ indexes, partition _N+1_ syncs in the background
   (single `tokio::spawn` or equivalent — one prefetch task, not a
   pool). When the indexer finishes _N_, it **deletes _N_'s local
   folder** (not optional — bounds disk at ~2 × partition*size)
   and then awaits the prefetch handle for \_N+1* (already done in
   the happy path → zero wait). Cleanup failure is a hard error
   (`?` propagation). This mirrors `backfill-bench`'s prefetch
   pattern, plus the mandatory cleanup step.
9. **Retry** — hand-rolled exponential backoff around the
   `aws s3 sync` subprocess: **3 attempts, 2s base delay, ×2
   multiplier, 30s cap**. Hardcoded module-level constants — not
   operator-tunable. Change the consts if the numbers drift. Parse
   errors are not retried — they indicate a data-shape bug and
   should surface immediately. Persist errors surface immediately
   too: schema / constraint violations are bugs, not transient
   failures.
   **Debug-first error stance (current):** parse / persist / post-
   sync missing-file errors **panic** rather than propagate a typed
   error. An operator running a multi-day backfill needs a stack
   trace and immediate crash, not a silent log-and-continue — the
   warn+continue path is the exact pattern we want to avoid while
   the write-path (0149) is still evolving. Revisit once the run
   has proven stable on staging.
10. **Observability — per-stage timing as a first-class feature** —
    `tracing` with the default human-readable formatter (structured
    fields rendered as `key=value`, not JSON — the runner is an
    operator CLI watched via `tail -f`, not a log-aggregator feed;
    the `status` subcommand covers the "query the run state" use
    case natively).
    **`--verbose` / `-v` flag** gates the live stream: without it,
    only `warn` and above print (so a non-verbose run is quiet
    except for retry warnings and panics); with it, per-ledger and
    per-partition info events flow. The flag is the only log-level
    dial — no env var, no separate format toggle.
    For **every ledger** (verbose only), emit an event with:
    sequence, partition, file size on disk, **parse duration**,
    **persist duration**. Decompress is intentionally not timed —
    deterministic zstd work on a fixed input carried no diagnostic
    signal relative to parse/persist and just cluttered the line.
    For **every partition** (verbose only), emit events at
    boundaries with: sync duration, total file count, total bytes
    on disk, aggregate parse / persist time, **min / max per-ledger
    total_ms** (parse + persist), wall-clock partition time,
    throughput (ledgers/sec). Partition boundaries are the natural
    checkpoint — no separate count-based progress event inside a
    partition. **Final run summary is `println!`, not tracing** —
    it always prints regardless of `--verbose`, so a quiet run
    still leaves one "what just happened" block: partitions
    processed, ledgers indexed / already-in-DB, total bytes, parse
    total, persist total, ledger time min / max, elapsed seconds.
    `status` is `println!`-only too — no tracing events in it,
    since it's a point-in-time CLI query, not a debug stream.
    Exit codes: `0` on success, panic on unrecoverable failure
    (process exits non-zero with a stack trace — see pkt 9).
    No CloudWatch metrics — operator-run CLI, not a deployed
    service. **Design intent:** the timing fields must make "this
    normally takes 500 ms but this run took 10 minutes" obvious
    from the logs without re-running.
11. **Pre-flight checks** — at startup, before touching any
    partition: (a) verify the `aws` binary exists on PATH (spawn
    `aws --version`, check exit 0); (b) verify DB connectivity with
    a trivial `SELECT 1`. Fail fast with a clear error if either
    check fails.
12. **README** — setup instructions, `aws` CLI prerequisite, temp dir
    disk-space expectations (one partition × 2 for prefetch overlap),
    measured partition throughput on a reference machine, cost notes,
    diff vs `backfill-bench` (why both exist).

### Out of scope

- **Concurrency / worker pool inside the indexer** — explicit
  non-goal. One ledger at a time, one partition at a time. If
  throughput later becomes a problem, revisit in a follow-up task
  — do not pre-optimize here.
- **Native `aws-sdk-s3`** — previously in scope, now out. `aws s3 sync`
  on a whole partition is the right tool for this job; a native SDK
  reimplementation of `sync` semantics is not worth the complexity.
  The runner treats the `aws` CLI as a dependency, like `zstd` or
  `psql`.
- **S3 artifact corpus** — task 0146 owns the artifact core. Not
  consumed here. Not emitted here.
- **Live Galexie ingestion** — a separate concern. Backfill and any
  future live path share only the parse-and-persist function.
- **Lambda / Fargate / ECS deployment** — operator CLI run from a
  workstation or single EC2 instance.
- **Replacing `backfill-bench`** — the bench stays as the reference /
  benchmark prototype. This runner is the production tool;
  coexistence is intentional.
- **Pre-Soroban ledgers** — scope starts at Protocol 20 go-live.
- **`DEFAULT` partition bootstrap** — unlike `backfill-bench`, the
  production runner assumes the partition-management Lambda (or the
  completion of 0149's partition-provisioning work) handles DB
  partition ranges authoritatively.
- **Re-persist on schema change** — 0149's problem; a re-run over a
  populated range is the fix.
- **Cross-region reads** — run the CLI in `us-east-1` (same region
  as the public archive) to keep ingress free.

## Implementation Plan

### Step 1 — Scaffold crate

`crates/backfill-runner/` binary crate. Add to workspace `members`.
Pull in dependencies (`indexer`, `xdr-parser`, `db`, `tokio`, `clap`,
`tracing`, `tracing-subscriber`, `chrono`, `sqlx`). No `aws-sdk-s3`.
`cargo check` + Nx targets register.

### Step 2 — CLI skeleton

`clap` derive-style CLI with `run` and `status` subcommands. Config
via CLI flags + env for `DATABASE_URL`, `--temp-dir`. No config file.
`tracing-subscriber` initialized up front with JSON-or-compact format
(env-selectable).

### Step 3 — Partition sync driver

`aws s3 sync --no-sign-request
s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/<partition>/
<temp-dir>/<partition>/` as a `tokio::process::Command`. Capture
stdout/stderr into tracing events, measure duration, surface exit
status. Partition helpers (`Partition`, `partitions_for_range`,
`s3_folder`, `local_folder`, `local_ledger_path`) **copied 1:1** from
`crates/backfill-bench/src/main.rs` into a new
`backfill-runner::partition` module. **Stage A resume** — none
required at this layer: the runner unconditionally invokes
`aws s3 sync` for each partition and relies on its native
idempotency (LIST-only when the dir is already complete).

### Step 4 — Sequential indexer for one partition

Given a synced partition directory: list `.xdr.zst` files, sort by
sequence, loop sequentially. For each: read file → decompress
(untimed) → timed `deserialize_batch` → timed `process_ledger` per
`LedgerCloseMeta`. Emit one structured event per ledger with
`bytes`, `parse_ms`, `persist_ms` + `seq`, `partition`. Track
per-partition min / max of `parse_ms + persist_ms` for the boundary
log and the final summary. Stage B resume check: skip sequences
already in the DB `HashSet`. Missing file post-sync → `assert!`
(panic) rather than warn+continue.

### Step 5 — Range planner + partition loop + background prefetch

Pre-flight: `aws --version` + `SELECT 1`. At startup: single
`SELECT sequence FROM ledgers WHERE sequence BETWEEN $1 AND $2`
builds the completed-sequences set. Enumerate partitions covering
the range, then **filter to a `todo: Vec<&Partition>`** via a
`partition_fully_done(p, start, end, &completed)` helper that
checks the clamped range `max(start, p.start)..=min(end, p.end)`.
Early-return if `todo` is empty. Main loop operates on `todo`:

1. Prime: foreground-sync `todo[0]`.
2. For each `(i, partition)` in `todo.iter().enumerate()`:
   `tokio::spawn` the sync for `todo.get(i + 1)` (background,
   `None` on the last iteration).
3. Index `partition` (sequential, Step 4). Stage B per-ledger
   skip runs inside — required for partial-partition DB state
   from a prior crash.
4. Delete the local partition folder (`tokio::fs::remove_dir_all`).
5. Await the prefetch handle; advance.

One prefetch in flight at any time — this is not a worker pool.
Cleanup is a hard error (`?` propagation) — a silent warn would
accumulate garbage. Disk footprint stays bounded at ~2 ×
partition_size (current being indexed + prefetch N+1 in flight).

### Step 6 — Retry around sync

Hand-rolled exponential backoff around the `aws s3 sync` subprocess
only (3 attempts default). No retry on parse / persist.

### Step 7 — Observability polish + README

Per-ledger and per-partition aggregate events. README documents
reference throughput per partition, disk-space footprint (2×
partition size during prefetch overlap), and diff vs
`backfill-bench`.

### Step 8 — Staging dry-run

One-partition window against staging DB. Verify:

- Stage A: re-running a partition is a LIST-only `aws s3 sync`
  (seconds, no GETs).
- Stage B resume: re-running skips all persists for sequences
  already in `ledgers`.
- `status` matches row counts.
- Timing events are present and legible.

### Step 9 — Production run

Full Soroban-era range → production DB. Monitor partition throughput
and per-stage timings. Disk footprint is bounded at ~2 ×
partition_size by mandatory cleanup-after-index, so there is no
retention dial to tune at this layer — if disk pressure appears,
it's a bug in the cleanup path, not a config knob.

## Acceptance Criteria

- [ ] `crates/backfill-runner/` builds and passes `nx run rust:build`,
      `nx run rust:test`, `nx run rust:lint`.
- [ ] Syncs from `aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`
      unsigned via `aws s3 sync --no-sign-request`, one partition at
      a time, into a local temp directory.
- [ ] Persists via `indexer::handler::process::process_ledger`. No
      reimplementation of write-path logic.
- [ ] Indexer is single-threaded: one ledger at a time, one partition
      at a time. No worker pool for ledger work.
- [ ] Exactly one partition prefetch is in flight at any time
      (background sync of _N+1_ while _N_ indexes).
- [ ] **Pre-sync partition skip** — at startup, after loading the
      completed-sequences set, partitions whose clamped range
      (`max(start, p.start)..=min(end, p.end)`) is fully in the set
      are filtered out and are **neither synced nor indexed**.
      Re-running a fully-done range does zero S3 work and zero
      `process_ledger` calls.
- [ ] **Stage A (sync)** — every partition that survives the
      pre-sync skip is synced via a single
      `aws s3 sync --no-sign-request` invocation. No marker, no
      manifest. Re-running a fully-synced partition dir is a cheap
      LIST-only call (native `aws s3 sync` idempotency).
- [ ] **Stage B resume** — inside a partition the pre-sync filter
      let through, re-running does not call `process_ledger` for
      sequences already in `ledgers` (handles mid-partition crashes).
- [ ] Resumes cleanly after SIGTERM / Ctrl-C — no duplicate rows, no
      missed ledgers.
- [ ] With `--verbose`, every ledger emits a structured `tracing`
      event (default human-readable formatter) with sequence,
      partition, file size, and **parse / persist durations**
      (decompress deliberately not timed). Every partition emits
      start/end events with **sync duration**, aggregate parse /
      persist totals, **min / max per-ledger total_ms**, wall-clock
      time, and throughput. Without `--verbose`, only `warn` / panic
      output is produced during the run.
- [ ] **Final run summary** is always printed (via `println!`,
      independent of `--verbose`): partitions processed, ledgers
      indexed / skipped, total bytes, parse total, persist total,
      ledger time min / max, elapsed seconds.
- [ ] Pre-flight: startup fails fast if `aws` is not on PATH or if
      DB is unreachable.
- [ ] Exit code `0` on full success; panic (non-zero + stack trace)
      on unrecoverable failure — no graceful typed-error exit while
      in debug-first stance.
- [ ] `status` accurately reports ingested / missing ledgers in a
      range **and** per-partition `range / indexed / pending`
      counts from the DB. The local temp dir is not inspected —
      cleanup-after-index makes it a transient signal with no
      long-term diagnostic value.
- [ ] After `index_partition(N)` returns `Ok`, the runner deletes
      the local partition folder. Disk footprint during a run stays
      bounded at ~2 × partition_size.
- [ ] README documents: `aws` CLI prerequisite, `--temp-dir` disk
      footprint (~2 × partition_size, bounded by cleanup-after-
      index), measured partition throughput, and the diff vs
      `crates/backfill-bench` (why both exist).
- [ ] Full Soroban-era range processed to production DB.

## Onboarding Notes

- **Fixed contract:** treat `indexer::handler::process::process_ledger`
  as a contract. Do not modify the write-path from this task. If you
  find a shape or behavior issue, raise it — 0149 owns the write-path
  body.
- **Reference crate:** `crates/backfill-bench/` is the prototype and
  the shape to follow. Lift: partition math, hex layout, range
  iteration, `aws s3 sync` subprocess, background prefetch pattern,
  sequential per-partition indexing. Do not lift: local `DEFAULT`
  partition bootstrap, minimal logging, dev-only shortcuts.
- **No concurrency:** if you catch yourself reaching for
  `tokio::task::JoinSet`, `mpsc`, or "N workers" — stop. The only
  background task in this runner is the single-slot partition
  prefetch.
- **Timing-first logging:** every stage that can be slow must be
  individually timed. Aggregate at the partition boundary. This is
  the entire point of the "productionize the wrapper" framing — an
  operator must be able to answer "where did the last hour go?" from
  logs alone.
- **Nx commands:** `pnpm nx build rust`, `pnpm nx test rust`,
  `pnpm nx lint rust`. Avoid global `cargo` — use the workspace's
  package manager.

## Risks / Notes

- **Disk pressure** — the runner deletes each partition's local
  folder after `index_partition(N)` returns `Ok`. During the run
  only **the partition being indexed plus the prefetch of N+1** are
  on disk, so the footprint stays bounded at ~2 × partition_size
  (couple of GB) regardless of how wide the range is. A crash
  leaves up to two partitions on disk; the two resume filters
  (pre-sync partition skip + per-ledger Stage B) handle restart
  cleanly — `aws s3 sync` fills in any partial folder on its own.
  Document the ~2× expectation in the README.
- **No cleanup-on-error (deliberate)** — cleanup runs only on the
  _successful_ path of each partition iteration. If
  `index_partition`, `sync_partition`, or the prefetch await
  returns `Err`, files stay on disk. Three reasons: (1) forensics
  — operator wants to inspect the partition that broke the run;
  (2) partial-sync resume is work already done — `aws s3 sync`
  fills in the missing tail on the next run, nuking the folder
  means redownloading from zero; (3) simpler error propagation
  (`?`), no Drop guards, no duplicated cleanup in every error
  arm. Disk bloat is bounded anyway (≤ 2 partitions on crash).
  Operator recovery if forensics aren't needed: `rm -rf
.temp/backfill-runner/`.
- **Cleanup ↔ pre-sync coupling** — cleanup-after-index is what
  _forces_ the pre-sync partition skip to exist. Without cleanup
  a re-run would see local folders and skip Stage A naturally;
  with cleanup the folders are gone, so the partition-level skip
  has to be computed from DB state before the sync runs. If
  cleanup is ever reverted, the pre-sync filter is still correct
  but becomes a micro-optimization rather than a necessity.
- **Ingress cost** — zero inside `us-east-1`. Run there.
- **Start ledger ambiguity** — `50_457_424` is community-sourced.
  Cross-verify with SDF opportunistically; a small leading gap is
  re-runnable cheaply.
- **`aws` CLI dependency** — the runner shells out to `aws`. Document
  the prerequisite in the README and fail fast with a clear error if
  the binary is missing.
- **Single-threaded throughput** — a deliberate trade. If the full
  historical range proves too slow at one ledger at a time, revisit
  in a follow-up; do not retrofit a worker pool inside this task.
- **Write-path stability** — if 0149 changes the `process_ledger`
  signature or semantics, this task rebases. Keep coupling at the
  single function-call layer.
- **Partition coverage (DB)** — the runner assumes partitioned tables
  have ranges provisioned for the backfill window. If 0149 / the
  partition-management Lambda doesn't cover the historical range at
  run time, the runner fails fast rather than auto-provisioning.
