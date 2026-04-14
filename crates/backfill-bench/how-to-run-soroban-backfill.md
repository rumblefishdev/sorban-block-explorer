# How to run full Soroban backfill (4 workers)

Full Soroban era: ledgers **50,457,424** to **~62,000,000** (~11.6M ledgers).

## Architecture

**4 Fargate tasks** writing to **1 shared RDS** instance.

- Each Fargate task runs `backfill-bench` for a different ledger range
- All tasks write to the same RDS PostgreSQL (same VPC, minimal latency)
- No merge needed — single database, no ID conflicts
- NFT classification uses WASM metadata from `wasm_interface_metadata` — all workers
  contribute to a shared classification pool in the same RDS
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

## 4. Monitor progress

Each Fargate task logs a summary on completion. Check CloudWatch for skipped/indexed counts.

To check DB progress:

```bash
psql $DATABASE_URL -c "SELECT MIN(sequence), MAX(sequence), COUNT(*) FROM ledgers"
```

## 5. Post-backfill: clean up NFT false positives

After **all 4 workers finish**, remove false-positive NFT records from contracts
classified as fungible or SAC. During backfill, some transfers may have been inserted
into `nfts` before WASM metadata was available for classification.

```sql
BEGIN;

-- 1. Sanity check: ensure classification is complete
SELECT COUNT(*) AS unclassified
FROM soroban_contracts
WHERE contract_type = 'other'
  AND contract_id IN (SELECT DISTINCT contract_id FROM nfts);
-- If >0: stop, investigate unclassified contracts first

-- 2. Remove false positives (fungible + SAC transfers mistakenly in nfts)
DELETE FROM nfts
WHERE contract_id IN (
    SELECT contract_id FROM soroban_contracts
    WHERE contract_type IN ('fungible', 'token')
);

COMMIT;

-- 3. Reclaim space
VACUUM ANALYZE nfts;
```

> The cleanup DELETE only removes records from contracts definitively classified as
> fungible or SAC. Unclassified contracts (`'other'`) are left untouched — investigate
> manually before purging.

## 6. Verify

```bash
psql $DATABASE_URL -c "
  SELECT contract_type, COUNT(*) AS nft_records
  FROM nfts n
  JOIN soroban_contracts c ON c.contract_id = n.contract_id
  GROUP BY contract_type;
"
```

After cleanup, all records in `nfts` should belong to contracts with
`contract_type = 'nft'`.
