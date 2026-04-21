# backfill-runner

Production-grade backfill of the Soroban-era Stellar pubnet archive into
Postgres. Streams `.xdr.zst` ledger objects from the public
`aws-public-blockchain` bucket, parses them, and persists to the ADR 0027
schema via `indexer::handler::process::process_ledger` — the shared
parse-and-persist contract. No S3 artifact, no local scratch directory,
no reimplementation of the write path.

## Prerequisites

- A reachable Postgres with the project schema migrated (ADR 0027).
- `DATABASE_URL` exported, or passed via `--database-url`.
- Run from `us-east-1` (same region as the public archive) to avoid
  cross-region ingress costs.

## Usage

```bash
# Backfill a sequence range.
cargo run -p backfill-runner -- run --start 50457424 --end 50460000

# Report ingested / missing ledgers for a range.
cargo run -p backfill-runner -- status --start 50457424 --end 50460000
```

### Flags

| Flag            | Default  | Notes                                          |
|-----------------|----------|------------------------------------------------|
| `--start`       | required | First ledger sequence (inclusive).             |
| `--end`         | required | Last ledger sequence (inclusive).              |
| `--workers`     | `4`      | Worker count (multi-worker pool is a follow-up — currently runs sequentially; flag reserved). |
| `--chunk-size`  | `100`    | Ledgers per worker job (reserved, see above).  |
| `--database-url`| env `DATABASE_URL` | Postgres DSN.                         |

### Start ledger

First Soroban-era ledger: `50_457_424` (2024-02-20, Protocol 20 go-live,
community-sourced). Cross-verify with SDF opportunistically; a small
leading gap is cheap to re-run.

## Resume & idempotency

On startup the runner queries `ledgers` once for the requested range and
builds a `HashSet<u32>` of completed sequences. The planner skips those
entirely — they are never fetched from S3. Combined with the existence
check inside `process_ledger`, re-running over a completed range is a
no-op (no writes, no failures).

## Retry policy

- **S3 fetch** — 3 attempts with exponential backoff (250 ms, 500 ms,
  1000 ms). Transient network / throttling errors retry.
- **`404 NoSuchKey`** — not retried; logged as an archive gap and
  skipped. Gaps do not fail the run.
- **Parse errors** — not retried. They indicate a data-shape bug and
  surface immediately.
- **Persist errors** — not retried. Schema / constraint violations are
  bugs, not transient failures.

## Observability

- Per-ledger `tracing` log line: sequence, compressed byte size, fetch
  duration, parse duration, persist duration.
- Periodic progress summary every 100 ledgers (and at run end) with
  done / total, percentage, throughput (ledgers/s), and ETA in seconds.
- Exit code non-zero on unrecoverable failure; archive gaps exit 0 and
  are listed in the final summary.

## Throughput

Reference throughput is **to be measured** on a `us-east-1` instance with
the production DB. Update this section after the first dry-run. Because
the pool is bounded by DB concurrent-write capacity, worker count should
be tuned to DB saturation, not CPU count.

## Diff vs `crates/backfill-bench`

Both crates target the **same sink** (Postgres, ADR 0027, via
`process_ledger`). `backfill-bench` is the prototype / benchmark;
`backfill-runner` is the operator-facing production tool. They coexist
intentionally.

| Axis                         | backfill-bench             | backfill-runner           |
|------------------------------|----------------------------|---------------------------|
| S3 fetch                     | `aws s3 sync` subprocess   | native `aws-sdk-s3`       |
| Scratch dir                  | `.temp/` on disk           | in-memory only            |
| Concurrency inside partition | sequential                 | worker pool (planned)     |
| Retry                        | none                       | exp backoff on S3 fetch   |
| Subcommands                  | single run                 | `run`, `status`           |
| Resume                       | re-walk from `--start`     | DB sequence-set filter    |
| `DEFAULT` partition bootstrap| yes (dev shortcut)         | no — assumes provisioned  |

## Known follow-ups

- Worker pool (`tokio::task::JoinSet` + bounded `mpsc`) wiring
  `--workers` / `--chunk-size`.
- Optional local watermark file for warm-start.
- `SIGTERM` graceful shutdown.
- Integration tests (partition math, resume query, end-to-end against
  a staging DB).

## Nx targets

```bash
pnpm nx build rust     # cargo build --workspace
pnpm nx test rust      # cargo test --workspace
pnpm nx lint rust      # cargo clippy --workspace -- -D warnings
```
