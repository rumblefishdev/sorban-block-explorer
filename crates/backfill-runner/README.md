# backfill-runner

Production-grade backfill of the Soroban-era Stellar pubnet archive into
Postgres. Syncs 64k-ledger partitions from the public
`aws-public-blockchain` bucket via `aws s3 sync`, decompresses +
deserializes each ledger, and persists to the ADR 0027 schema via
`indexer::handler::process::process_ledger` — the shared parse-and-persist
contract. No reimplementation of the write path.

## Prerequisites

- The `aws` CLI on `PATH` (subprocess driver — no native SDK dependency).
  Startup fails fast if `aws --version` can't run.
- A reachable Postgres with the project schema migrated (ADR 0027).
  Startup fails fast if `SELECT 1` fails.
- `DATABASE_URL` exported, or passed via `--database-url`.
- Run from `us-east-1` (same region as the public archive) to avoid
  cross-region ingress costs.
- Local scratch disk: **~2 × partition_size** (a couple of GB). The runner
  keeps at most the partition being indexed plus the prefetched N+1 on
  disk; each partition's folder is deleted after it fully indexes.

## Usage

```bash
# Backfill a sequence range.
cargo run -p backfill-runner -- run --start 50457424 --end 50460000

# Report per-partition progress for a range.
cargo run -p backfill-runner -- status --start 50457424 --end 50460000
```

### Flags

| Flag             | Default                  | Notes                                               |
|------------------|--------------------------|-----------------------------------------------------|
| `--start`        | required                 | First ledger sequence (inclusive).                  |
| `--end`          | required                 | Last ledger sequence (inclusive).                   |
| `--database-url` | env `DATABASE_URL`       | Postgres DSN (required if env not set).             |
| `--temp-dir`     | `.temp/backfill-runner`  | Local scratch dir (env `BACKFILL_TEMP_DIR`).        |
| `--verbose`/`-v` | off                      | Enable per-ledger + per-partition info logs. Without it only warnings print during the run. |

### Start ledger

First Soroban-era ledger: `50_457_424` (2024-02-20, Protocol 20 go-live,
community-sourced). Cross-verify with SDF opportunistically; a small
leading gap is cheap to re-run.

## Shape

One partition at a time, sequential per-ledger inside. Exactly **one**
background task: a single-slot prefetch of partition N+1 running while
partition N indexes. No worker pool, no `JoinSet` of indexer tasks —
concurrency inside the indexer is explicitly out of scope.

After partition N finishes indexing, its local folder is deleted before
awaiting the N+1 prefetch. This bounds disk at ~2 × partition_size
regardless of range width.

## Resume & idempotency

The DB `ledgers` table is the sole source of truth — no state file, no
manifest, no marker.

Two resume filters run against the `HashSet<u32>` of completed sequences
built at startup:

1. **Pre-sync partition skip** — partitions whose clamped range
   (`max(start, p.start)..=min(end, p.end)`) is fully present in the set
   are filtered out and neither synced nor indexed. Re-running a
   fully-done range does zero S3 work and zero `process_ledger` calls.
2. **Per-ledger skip (inside a partition)** — for partitions that survive
   the pre-sync filter, `process_ledger` is skipped for any sequence
   already in the set. Handles mid-partition crashes where the partition
   is only partially in DB.

`aws s3 sync` itself is idempotent — a call against a fully-synced dir is
a LIST with no GETs — so there is no Stage A marker or file-count check.
A partial dir from a crashed run gets filled in on the next sync.

## Retry policy

- **`aws s3 sync`** — 3 attempts, 2s base delay, ×2 multiplier, 30s cap.
  Hardcoded constants in `sync.rs`; change them if the numbers drift.
- **Parse / persist errors** — not retried. Parse failures indicate a
  data-shape bug; schema / constraint violations are write-path bugs.
  Both surface immediately.
- **Missing file post-sync** — panics (`assert!`). A file absent after a
  successful `aws s3 sync` means either an archive gap or a sync bug;
  both are worth a stack trace, not a silent skip. Debug-first stance
  for the duration of 0149's write-path churn — revisit once stable.

## Observability

The `run` subcommand emits a live human-readable `tracing` stream (default
formatter). `--verbose` / `-v` is the only log-level dial — without it
the filter is `warn`, so a quiet run shows only retry warnings and
panics; with it, per-ledger and per-partition `info` events flow.
Operator-facing — meant for `tail -f` while a long backfill runs. The
`status` subcommand is the structured "how far along" query; the two do
not overlap.

Per-ledger event `ledger ingested` (verbose only):
`seq`, `partition`, `bytes`, `parse_ms`, `persist_ms`. Decompression is
intentionally **not** timed — deterministic zstd work on a fixed input
carried no diagnostic signal relative to parse/persist and was just
noise on the line. Per-partition `partition indexing complete` (verbose
only): aggregate parse / persist totals, **min / max per-ledger
total_ms**, wall clock, throughput (ledgers/s). Sync layer emits
`running aws s3 sync`, `partition sync complete` (duration + file count
+ bytes), and `warn` on each retry.

**Final run summary** is always printed via `println!` regardless of
`--verbose`, so a quiet run still leaves one "what just happened"
block: partitions processed, ledgers indexed / already in DB, parse
total, persist total, ledger time min / max, elapsed seconds.

Exit code `0` on success; unrecoverable failures **panic** (non-zero
exit + stack trace) rather than return a typed error, per the
debug-first stance noted in the Retry section.

### `status` output

```
range: 50457424..=50460000   partitions: 1
   partition       indexed / range    pending   progress
    50425856            2577 / 2577         0     100.0%
----------------------------------------------------------
       total            2577 / 2577         0     100.0%
```

`indexed` / `range` and `pending` are counted against the **clamped**
range per partition — edge partitions that stick out of the requested
window only count the in-window slice. `progress` is `indexed / range`
as a percentage.

## Disk footprint

Bounded at ~2 × partition_size by mandatory cleanup-after-index. A crash
leaves at most two partitions on disk (the one being indexed + the N+1
prefetch); both are reclaimed on the next successful iteration, and
`aws s3 sync` patches up any partial folder. On error, cleanup is
**deliberately skipped** — the broken partition stays on disk for
forensics, and `aws s3 sync` on retry fills in any missing tail instead
of re-downloading from zero. If forensics aren't needed after a failure,
`rm -rf .temp/backfill-runner/` is the recovery.

## Throughput

Reference throughput per partition is **to be measured** on a `us-east-1`
instance against the production DB. Update this section after the first
dry-run.

## Diff vs `crates/backfill-bench`

Both crates target the **same sink** (Postgres, ADR 0027, via
`process_ledger`) and use the **same unit of work** (one 64k-ledger
partition via `aws s3 sync`). `backfill-bench` is the prototype /
benchmark; `backfill-runner` is the operator-facing production tool.
They coexist intentionally.

| Axis                     | backfill-bench             | backfill-runner                      |
|--------------------------|----------------------------|--------------------------------------|
| S3 fetch                 | `aws s3 sync` subprocess   | `aws s3 sync` subprocess             |
| Scratch dir              | `.temp/`                   | `.temp/backfill-runner/` (flag)      |
| Cleanup after index      | no                         | yes (disk bounded at ~2 × partition) |
| Concurrency              | sequential + prefetch      | sequential + prefetch (same)         |
| Retry                    | none                       | 3× exp backoff on `aws s3 sync`      |
| Subcommands              | single run                 | `run`, `status`                      |
| Pre-flight checks        | none                       | `aws --version` + `SELECT 1`         |
| Resume — partition level | none                       | pre-sync skip against DB             |
| Resume — ledger level    | none                       | per-ledger skip against DB           |
| Per-stage timing logs    | minimal                    | every ledger + per-partition totals  |
| `DEFAULT` partition boot | yes (dev shortcut)         | no — assumes provisioned             |

## Nx targets

```bash
pnpm nx build rust     # cargo build --workspace
pnpm nx test rust      # cargo test --workspace
pnpm nx lint rust      # cargo clippy --workspace -- -D warnings
```
