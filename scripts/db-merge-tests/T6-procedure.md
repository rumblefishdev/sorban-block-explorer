# T6 — Scale smoke test (≥10M-ledger snapshot pair)

Manual procedure. **Not** part of `run-all.sh` — too long to include in
CI-style runs and resource-heavy enough that it deserves dedicated
scheduling.

## Acceptance threshold (AC #16)

- Wall-clock per ~10M-ledger snapshot ingest: **≤ 4 hours**
- Peak temp space (`.temp/db-merge-backups/` + snapshot-source volume):
  **≤ 30% of source dump size**
- Diff matches between sequential-truth and merged-target: **17/17**

Adjust thresholds in the task notes after the first real run records
actuals.

## Setup (assumes ≥1 TB free disk + ≥32 GB RAM)

Pick a real mainnet range:

- recommended size: **2 × 10M ledgers** (one per laptop) — total ~50M-row
  `transactions` and ~150M-row `operations_appearances` per snapshot
- coordinated start: ledger `S` to `S + 20_000_000`, split at midpoint

```bash
START=50000000      # adjust to a real ledger
MID=$((START + 10_000_000))
END=$((START + 20_000_000))
```

## Step-by-step

1. **Bring up DBs** (5 containers, plenty of disk for each):

   ```bash
   COMPOSE_PROFILES=db-merge docker compose up -d \
     postgres-truth postgres-laptop-a postgres-laptop-b \
     postgres-merge postgres-snapshot-source
   ```

2. **Migrate + partition** all 5 (use `_lib.sh::migrate` /
   `create_partitions`, or run `db-partition-mgmt` for monthly
   partitions if you want to test that branch).

3. **Backfill** (these run in parallel — different machines/cores per laptop):

   ```bash
   # On any host: truth (sequential, full range) — the slow one
   time backfill-runner run --database-url postgres://...:5433/... \
     --start $START --end $END

   # On laptop A
   time backfill-runner run --database-url postgres://...:5434/... \
     --start $START --end $MID

   # On laptop B
   time backfill-runner run --database-url postgres://...:5435/... \
     --start $((MID+1)) --end $END
   ```

4. **Dump the laptops**:

   ```bash
   time pg_dump --format=custom -f laptop-a.dump postgres://...:5434/...
   time pg_dump --format=custom -f laptop-b.dump postgres://...:5435/...
   du -sh laptop-{a,b}.dump      # for the temp-space ratio later
   ```

5. **Merge** (the test):

   ```bash
   # Time the whole sequence
   time db-merge ingest laptop-a.dump \
     --target-url postgres://...:5436/... \
     --snapshot-source-url postgres://...:5437/...

   time db-merge ingest laptop-b.dump \
     --target-url postgres://...:5436/... \
     --snapshot-source-url postgres://...:5437/...

   time db-merge finalize --target-url postgres://...:5436/...
   ```

6. **Diff** (correctness):
   ```bash
   db-merge diff \
     --left  postgres://...:5433/... \
     --right postgres://...:5436/...
   ```
   Expected: `17/17 tables match`.

## What to record

After the run, append to task 0186 notes:

```
T6 actuals (date: YYYY-MM-DD):
- Range: [START, END] (~XXM ledgers)
- Backfill truth wall-clock:    Xh Ym
- Backfill laptop-a wall-clock: Xh Ym
- pg_dump laptop-a:             X GB, Ys
- pg_dump laptop-b:             X GB, Ys
- ingest laptop-a wall-clock:   Xh Ym
- ingest laptop-b wall-clock:   Xh Ym
- finalize wall-clock:          Ms
- Pre-merge backup size:        X GB
- snapshot-source volume peak:  X GB
- Temp space ratio:             N% (peak temp / source dump)
- Diff result:                  17/17 / N mismatches
- Peak RSS db-merge:             MB
```

If thresholds are violated, file follow-up tasks for the offending
phase (most likely Phase D step 10 — the FK-rewrite JOINs on
appearance tables — needs the B-tree index sanity check).

## Monitoring while it runs

In another terminal:

```bash
# Watch target growth
watch -n 30 "psql ...:5436/... -c \"
  SELECT relname, n_live_tup
    FROM pg_stat_user_tables
   ORDER BY n_live_tup DESC LIMIT 10\""

# Watch temp dir size
watch -n 30 "du -sh .temp/db-merge-backups/"

# Snapshot source volume
docker system df -v | grep snapshot-source
```
