# How to run full Soroban backfill (4 workers)

Full Soroban era: ledgers **50,457,424** to **~62,000,000** (~11.6M ledgers).

## Architecture

**4 Fargate tasks** writing to **1 shared RDS** instance.

- Each Fargate task runs `backfill-bench` for a different ledger range
- All tasks write to the same RDS PostgreSQL (same VPC, minimal latency)
- No merge needed — single database, no ID conflicts
- `nft_candidates` resolve works immediately after backfill (WASM metadata from all
  workers already in one place)
- 4 connections to RDS is negligible load

**Assumptions:**
- Database is clean (empty) before starting
- RDS and Fargate tasks are in the same VPC
- `DATABASE_URL` env var points to the shared RDS instance

## 1. Prerequisites

Ensure RDS is running and migrated:

```bash
psql $DATABASE_URL -c "SELECT 1"
```

If fresh database, run migrations first (from a machine with DB access):

```bash
DATABASE_URL=<rds-connection-string> npm run db:migrate
```

## 2. Build and push Docker image

```bash
cargo build --release -p backfill-bench
# Build and push container image to ECR (adjust for your registry)
```

## 3. Launch 4 Fargate tasks

Each task gets a different ledger range, all sharing the same `DATABASE_URL`:

| Task | Start | End | Ledgers |
|------|-------|-----|---------|
| 1 | 50,457,424 | 53,357,423 | ~2.9M |
| 2 | 53,357,424 | 56,257,423 | ~2.9M |
| 3 | 56,257,424 | 59,157,423 | ~2.9M |
| 4 | 59,157,424 | 62,057,423 | ~2.9M |

> Adjust Task 4 `--end` to current ledger height before running.

Each Fargate task runs:

```bash
backfill-bench --start <start> --end <end> --database-url $DATABASE_URL
```

> Each task downloads partitions from S3 independently. No AWS credentials needed
> for S3 (public bucket), but tasks need network access to RDS.

## 5. Monitor progress

Each Fargate task logs a summary on completion. Check CloudWatch for skipped/indexed counts.

To check DB progress:

```bash
psql $DATABASE_URL -c "SELECT MIN(sequence), MAX(sequence), COUNT(*) FROM ledgers"
```

## 6. Post-backfill: resolve NFT candidates

After **all 4 workers finish**, run the resolve script to move confirmed NFTs
from `nft_candidates` staging table to `nfts`:

```sql
BEGIN;

INSERT INTO nfts (contract_id, token_id, owner, transaction_hash, event_kind,
                  ledger_sequence, created_at)
SELECT nc.contract_id, nc.token_id, nc.owner, nc.transaction_hash, nc.event_kind,
       nc.ledger_sequence, nc.created_at
FROM nft_candidates nc
JOIN contracts c ON c.contract_id = nc.contract_id
JOIN wasm_interface_metadata wim ON wim.wasm_hash = c.wasm_hash
WHERE wim.metadata @> '{"functions": [{"name": "token_uri"}]}'
   OR wim.metadata @> '{"functions": [{"name": "owner_of"}]}';

DELETE FROM nft_candidates
WHERE contract_id IN (
  SELECT c.contract_id
  FROM contracts c
  JOIN wasm_interface_metadata wim ON wim.wasm_hash = c.wasm_hash
);

COMMIT;
```

> This moves NFT-classified candidates to `nfts` and removes all resolved entries
> (both NFT and fungible). Candidates with no WASM metadata remain for manual review.

## 7. Verify

```bash
psql $DATABASE_URL -c "
  SELECT 'nfts' AS table_name, COUNT(*) FROM nfts
  UNION ALL
  SELECT 'nft_candidates', COUNT(*) FROM nft_candidates;
"
```

`nft_candidates` should be near zero. Any remaining entries are contracts without
WASM metadata — review manually or wait for metadata to become available.
