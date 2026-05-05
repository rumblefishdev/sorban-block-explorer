# db-merge test corpus (T1–T6)

Bash test harness for the merge script. Implements the test plan from
[task 0186 §Step 5](../../lore/1-tasks/active/0186_FEATURE_db-merge-multi-laptop-snapshots.md).

## Quick start

```bash
cargo build -p db-merge
bash scripts/db-merge-tests/run-all.sh
```

`run-all` brings up the 5 db-merge containers, runs T1–T5 sequentially,
and tears everything down on exit (regardless of pass/fail).

Single test:

```bash
bash scripts/db-merge-tests/T3-two-snapshot-chrono.sh
```

## What each test verifies

| Test                                   | Verifies                                                                                                                         | AC  |
| -------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- | --- |
| **T1** First-snapshot edge case        | Empty target ← single snapshot ⇒ `diff(merge, source)` = 17/17. All `merge_remap` rows are degenerate (no conflict-update path). | #11 |
| **T2** Single-snapshot reproducibility | A sequentially-backfilled DB (truth) and a snapshot-merged DB of the same range are logically identical.                         | #12 |
| **T3** Two-snapshot chronological      | Truth (sequential A+B) vs merge (snapshot-A then snapshot-B then finalize) ⇒ 17/17. **The actual end-to-end correctness test.**  | #13 |
| **T4** Idempotency                     | Replaying both snapshots after the initial merge with `--allow-overlap` is a strict no-op (zero new rows; diff still 17/17).     | #14 |
| **T5** Wrong-order rejected            | Ingesting an earlier-range snapshot after a later one aborts on the chronological-only preflight gate.                           | #15 |
| **T6** Scale smoke (≥10M ledgers)      | Manual procedure — see `T6-procedure.md`.                                                                                        | #16 |

## Synthetic vs real fixtures

The provided seeds (`seed-laptop-{a,b}.sql`, `seed-truth.sql`) are
**synthetic** — they model the exact rows two laptops on disjoint
ledger ranges [1..3] and [4..6] would produce, with deliberate overlap
on `accounts.alice` and `soroban_contracts.C1` to exercise the remap
conflict-update path.

Synthetic suffices to validate the merge logic. **For production
validation use real backfilled fixtures**:

```bash
# Pick a range with reasonable activity (~10k ledgers around any block
# with Soroban + SAC + NFT mints + LP activity).
START=55000000   # adjust
MID=55005000
END=55010000

# Backfill truth (full range) — single laptop, sequential.
backfill-runner run --database-url $URL_TRUTH --start $START --end $END

# Backfill laptop A (lower half).
backfill-runner run --database-url $URL_LAPTOP_A --start $START --end $MID

# Backfill laptop B (upper half).
backfill-runner run --database-url $URL_LAPTOP_B --start $((MID+1)) --end $END

# Now T1-T5 work against real data — re-run scripts; they pick up the
# laptop-a/laptop-b state via `seed` (skip the SQL seeds and manually
# pg_dump the laptop DBs into the fixture path the tests expect).
```

`backfill-runner` pulls from `aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`
(public S3 bucket, no credentials needed for read).

## Architecture notes

- `_lib.sh` — every test sources this; provides `bring_up`, `reset_db`,
  `migrate`, `create_partitions`, `seed`, `dump_db`, `assert_diff_match`,
  `final_teardown`. Encapsulates URL conventions and the
  drop-volume-recreate-migrate-partition reset pattern.
- Each test does its own `fresh_setup` — guarantees isolation between tests.
- Default partitions are created manually (mirrors what `db-partition-mgmt`
  would do in prod; preflight requires them).
- Hash assertion is via `db-merge diff` — same harness the real
  correctness check uses.

## Why `--allow-overlap` exists

T4 (idempotency) replays already-merged snapshots. Without
`--allow-overlap`, the chronological-only preflight gate aborts because
`source MIN <= target MAX`. The flag bypasses just that one check;
all other gates (migration parity, partition layout, CHECKs) remain
active. The flag is **not** for normal use — only for replay testing.
