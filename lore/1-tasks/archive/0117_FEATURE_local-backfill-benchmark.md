---
id: '0117'
title: 'Local backfill benchmark: stream from Stellar public S3 to local Postgres'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0030', '0113']
tags: [priority-high, effort-medium, layer-backend]
links: []
history:
  - date: '2026-04-09'
    status: backlog
    who: fmazur
    note: 'Task created — team wants to benchmark indexer performance locally before committing to AWS backfill costs'
  - date: '2026-04-09'
    status: active
    who: fmazur
    note: 'Activated task'
  - date: '2026-04-13'
    status: done
    who: fmazur
    note: >
      Implemented backfill-bench CLI crate. PR #89 merged. Key work: exposed
      process_ledger from indexer crate, built stream-and-delete flow with
      reqwest, batch UNNEST persistence with Rust-side COALESCE merge,
      JoinSet-based concurrency. 7 commits.
---

# Local backfill benchmark: stream from Stellar public S3 to local Postgres

## Summary

CLI tool that streams XDR files from the Stellar public S3 bucket (`s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`), indexes them through the existing parser/persist pipeline, and writes to local PostgreSQL. Each file is deleted from disk immediately after indexing to avoid filling local storage. Measures throughput to inform decisions about AWS backfill (task 0030).

## Context

Team decision: before running backfill on Fargate (task 0030), we need to know how long indexing takes per ledger and extrapolate to full history. The Stellar public S3 bucket has all mainnet ledger data in the same format Galexie produces — no authentication required (`--no-sign-request`).

Public bucket: `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`
Format: `{hex}--{ledger}.xdr.zst`, 1 ledger per file, zstd compressed.

## Implementation Plan

### Step 0: Expose `process_ledger` from indexer crate

The indexer is currently a binary-only crate (`src/main.rs`). To reuse `process_ledger()` without duplicating code:

1. Add `crates/indexer/src/lib.rs` with `pub mod handler;`
2. Make `handler` module and `handler::process::process_ledger` public
3. No behavioral changes — Lambda works exactly as before

This is the only modification to existing code. It adds `pub` visibility, nothing else.

### Step 1: Create CLI binary `crates/backfill-bench/`

New Rust binary crate in the workspace. Dependencies:

- `indexer` (library) — `process_ledger()` via the lib.rs exposed in Step 0
- `xdr-parser` — `decompress_zstd()`, `deserialize_batch()`
- `db` — connection pool
- `reqwest` — HTTPS download from public S3 (no AWS SDK needed)
- `clap` — CLI argument parsing

CLI args:

- `--start <ledger>` — first ledger to index (inclusive)
- `--end <ledger>` — last ledger to index (inclusive)
- `--database-url <url>` — Postgres connection string (default: `DATABASE_URL` env var)

### Step 2: Stream-and-delete flow

For each ledger in range:

1. Check if ledger already exists in local DB (`SELECT EXISTS(... FROM ledgers WHERE sequence = $1)`) — if yes, skip
2. Download single file from public S3 via HTTPS to a temp file (no AWS SDK needed — public bucket, use `reqwest`)
3. Read and decompress zstd
4. Parse XDR and persist to Postgres (reuse `process_ledger`)
5. Delete the downloaded file from disk
6. Log progress every 10 ledgers: current sequence, elapsed time, avg ms/ledger, skipped count

URL pattern:

```
https://aws-public-blockchain.s3.us-east-2.amazonaws.com/v1.1/stellar/ledgers/pubnet/{partition_dir}/{hex}--{ledger}.xdr.zst
```

Where:

- `partition_dir` = `{hex(u32::MAX - partition_start)}--{partition_start}-{partition_end}`
- `partition_start` = `ledger - (ledger % 64000)`
- File hex = `hex(u32::MAX - ledger)`, uppercase, 8 chars

### Step 3: Measure and report

At the end, print summary:

- Start time and end time
- Requested range (--start to --end)
- Ledgers indexed (newly processed)
- Ledgers skipped (already in DB)
- Total elapsed time
- Average ms per indexed ledger (excluding skipped)
- Average XDR file size in KB (compressed, as downloaded)

## Acceptance Criteria

- [x] CLI binary builds and runs locally
- [x] Downloads XDR files from Stellar public S3 (no AWS credentials needed)
- [x] Indexes and persists to local PostgreSQL (docker-compose)
- [x] Downloaded XDR files deleted from disk after indexing — no accumulation
- [x] Progress logging every 10 ledgers
- [x] Final report with timing stats
- [x] Skips already-indexed ledgers without re-downloading
- [x] Successfully indexes 1000 ledgers from a recent range

## Notes

- No need for parallelism in v1 — sequential is fine for benchmarking
- `reqwest` with HTTPS is simpler than AWS SDK for public bucket access
- DB migrations must be run first (`npm run db:migrate`)
- This is a dev tool, not production code — doesn't need to be in CI/CD
