# backfill-bench

Local backfill benchmark — streams XDR files from the Stellar public S3 bucket and indexes them into a local PostgreSQL database.

## Prerequisites

- Docker running (for local PostgreSQL)
- Database migrated

```bash
docker compose down -v --rmi local
docker compose up -d
npm run db:migrate
```

## Usage

```bash
cargo run -p backfill-bench -- --start <ledger> --end <ledger>
```

### Examples

Index 1000 recent ledgers:

```bash
cargo run --release -p backfill-bench -- --start 62016000 --end 62016099
```

Index 100 ledgers with explicit database URL:

```bash
cargo run -p backfill-bench -- --start 62015000 --end 62015099 --database-url postgres://postgres:postgres@127.0.0.1:5432/soroban_block_explorer
```

> **Tip:** Use `127.0.0.1` instead of `localhost` in the connection string to avoid DNS resolution delays on some systems (IPv6/IPv4 fallback).

### Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `--start` | yes | First ledger to index (inclusive) |
| `--end` | yes | Last ledger to index (inclusive) |
| `--database-url` | no | Postgres connection string (default: `DATABASE_URL` env var) |

## What it does

For each ledger in the range:

1. Checks if ledger already exists in local DB — skips if yes
2. Downloads `.xdr.zst` file from `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` (public, no AWS credentials needed)
3. Decompresses and parses XDR in memory
4. Persists to local PostgreSQL (same pipeline as the Lambda indexer)

After completion, prints a summary:

```
=== Backfill complete ===
Started:    2026-04-09 12:00:00
Finished:   2026-04-09 12:05:32
Range:      62015000 - 62015999
Indexed:    847
Skipped:    153
Elapsed:    332.4s
Avg:        392 ms/ledger
Avg file:   187.3 KB
```

## Ledger ranges

| Range | Ledgers | Period | Avg file size | Description |
|-------|---------|--------|---------------|-------------|
| 0 – 50,457,424 | ~50.5M | Sep 2015 – Feb 2024 | 400B – 220KB | Pre-Soroban (classic Stellar) |
| **50,457,424 – present** | **~11.6M** | **Feb 2024 – present** | **~180 KB** | **Soroban era (Protocol 20+)** |

Soroban smart contracts (Protocol 20) were activated on mainnet at **ledger 50,457,424** (February 20, 2024, 17:00 UTC).

For a Soroban-only block explorer, use `--start 50457424`.

Full details: `lore/3-wiki/stellar-pubnet-ledger-archive.md`

## Data source

Stellar public ledger data hosted on AWS Open Data:
`s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`

No authentication required (`--no-sign-request`).
