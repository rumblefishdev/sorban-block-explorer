---
title: 'Rollout commands — steps 3, 5, 6 and gate 7a, ready to paste'
type: generation
status: developing
spawned_from: ../README.md
spawns: []
tags: ['rollout', 'backfill', 'clickhouse', 'hetzner']
links:
  - ../../../../docs/runbooks/backfill_derived_table_reparse_hetzner.md
history:
  - date: 2026-09-06
    status: developing
    who: karolkow
    note: >
      Exact commands for the map owner's half of rollout step 3 (tables +
      snapshot), 5 (ship the binary) and 6 (the targeted backfill), plus the
      gate-7a coverage queries. Adapted from the Hetzner re-parse runbook; the
      only differences are the three tables and `--only`.
---

# Rollout commands (task 0540)

Follows `docs/runbooks/backfill_derived_table_reparse_hetzner.md`, flavour B
(from-S3 re-parse). Every command below is the **map owner's** to run on the
box; nothing here is run by an agent. Order is the T08 sequence: 2 → 3 → 4 → 5
→ 6 → 7.

## Step 2 — before anything

```bash
# BOX
df -h / | tail -1                       # do not start under ~300 GB free (runbook §2)
pgrep -af 'bf-loop|backfill-runner' | wc -l   # 0 = no other backfill running
```

New tables are ~56–73 GB (estimate) plus s5cmd scratch ≈ 2 × 11.6 GB per worker.

## Step 3 — create the three tables, then snapshot

Created **before** the indexer that writes them deploys (the driver validates
the row struct against `DESCRIBE`; a missing table fails every insert
client-side — task 0310). `CREATE TABLE IF NOT EXISTS`, so re-running is safe.
Verbatim from `crates/db-clickhouse/schema/init.sql` at commit `c62d12dd`.

```bash
# BOX
docker exec -i app-clickhouse-1 clickhouse-client --multiquery <<'SQL'
CREATE TABLE IF NOT EXISTS asset_transfers (
    ledger_sequence    Int64                   CODEC(ZSTD(3)),
    application_order  Int16                   CODEC(ZSTD(3)),
    op_index           Int16                   CODEC(ZSTD(3)),
    event_pos_in_op    Int16                   CODEC(ZSTD(3)),
    event_index        Int16                   CODEC(ZSTD(3)),
    asset_id           Int64                   CODEC(ZSTD(3)),
    amount             Nullable(Int128)        CODEC(ZSTD(3)),
    from_id            Nullable(Int64)         CODEC(ZSTD(3)),
    from_kind          LowCardinality(String)  CODEC(ZSTD(3)),
    from_muxed_id      Nullable(UInt64)        CODEC(ZSTD(3)),
    to_id              Nullable(Int64)         CODEC(ZSTD(3)),
    to_kind            LowCardinality(String)  CODEC(ZSTD(3)),
    to_muxed_id        Nullable(UInt64)        CODEC(ZSTD(3)),
    verb               LowCardinality(String)  CODEC(ZSTD(3))
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (ledger_sequence, application_order, op_index, event_pos_in_op)
SETTINGS index_granularity = 512;

CREATE TABLE IF NOT EXISTS transaction_memos (
    ledger_sequence    Int64                   CODEC(ZSTD(3)),
    application_order  Int16                   CODEC(ZSTD(3)),
    memo_type          LowCardinality(String)  CODEC(ZSTD(3)),
    memo               String                  CODEC(ZSTD(3))
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (ledger_sequence, application_order);

CREATE TABLE IF NOT EXISTS soroban_event_ops (
    ledger_sequence    Int64                   CODEC(ZSTD(3)),
    transaction_id     Int64                   CODEC(ZSTD(3)),
    event_index        Int16                   CODEC(ZSTD(3)),
    op_index           Int16                   CODEC(ZSTD(3)),
    event_pos_in_op    Int16                   CODEC(ZSTD(3))
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (ledger_sequence, transaction_id, event_index);
SQL

# confirm the DESCRIBE the driver will see
docker exec app-clickhouse-1 clickhouse-client -q "DESCRIBE asset_transfers" | head -20
```

```bash
# BOX — pre-backfill snapshot (ASYNC dodges the 300 s client receive_timeout)
docker exec app-clickhouse-1 clickhouse-client -q \
  "BACKUP DATABASE default TO Disk('backups', 'snapshot_pre_0540_backfill_$(date +%Y%m%d)') ASYNC"
docker exec app-clickhouse-1 clickhouse-client -q \
  "SELECT name, status, error, formatReadableSize(total_size) FROM system.backups ORDER BY start_time DESC LIMIT 1"
```

## Step 4 — deploy the indexer, same window as the `ALTER`

From the laptop, per `docs/deployment.md`:

```bash
make -C infra diff-production
make -C infra deploy-production-compute
```

and in the **same window**, because the `net_settled` struct field is gone
since `a2f8c5a7`:

```bash
# BOX
docker exec app-clickhouse-1 clickhouse-client -q \
  "ALTER TABLE operation_asset_appearances DROP COLUMN net_settled"
```

Note the ledger the new indexer first writes — call it **L₀** — from the
Lambda logs or:

```bash
docker exec app-clickhouse-1 clickhouse-client -q \
  "SELECT min(ledger_sequence) FROM asset_transfers"
```

`L₀` is the backfill's `END`. The overlap around it is safe: both writers are
the same commit, rows are byte-identical, the RMT collapses them.

## Step 5 — ship the binary

Built on the laptop with `cargo zigbuild --release -p backfill-runner --bin
backfill-runner --target x86_64-unknown-linux-gnu.2.31` from the same commit
as the deployed indexer (`c62d12dd` or later on the branch).

```bash
# LAPTOP
scp target/x86_64-unknown-linux-gnu/release/backfill-runner deploy@ch-prod-01:~/backfill-runner
```

```bash
# BOX — smoke: the flag MUST be there (a pre-flag binary wrote 0 rows in 2026-07)
chmod +x ~/backfill-runner
~/backfill-runner --version
~/backfill-runner --help | grep -- '--only'
```

## Step 6 — the targeted backfill

`~/meta.env` as in the runbook (`CLICKHOUSE_URL=http://localhost:8123`,
user, password from `/srv/app/.env`, `BIN`, `S5CMD`). The worker script is
the runbook's `bf-loop16.sh` with one change — the global `--only` flag
before `run`:

```bash
#!/usr/bin/env bash
# BOX — ~/bf-loop-0540.sh <start> <end> <watermark-file>
set -uo pipefail
START=$1; END=$2; WM=$3; F=64000; P=16000
DATA="${DATA:?}"; BIN="${BIN:?}"; S5CMD="${S5CMD:-s5cmd}"
: "${CLICKHOUSE_URL:?}"
ONLY=asset_transfers,transaction_memos,soroban_event_ops
BUCKET="s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet"
export AWS_REGION=us-east-2 AWS_DEFAULT_REGION=us-east-2
export STELLAR_NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"
mkdir -p "$DATA"
fstart=$(( START - (START % F) ))
for (( f0=fstart; f0<=END; f0+=F )); do
  f1=$(( f0+F-1 )); lo=$(( f0>START?f0:START )); hi=$(( f1<END?f1:END ))
  [ -s "$WM" ] && [ "$(cat "$WM")" -ge "$hi" ] && continue          # already done
  folder="$(printf '%08X' $((4294967295-f0)))--${f0}-${f1}"
  dir="$DATA/$folder"; mkdir -p "$dir"; list="$(mktemp)"
  for (( s=f0; s<=f1; s++ )); do
    printf 'cp "%s/%s/%08X--%d.xdr.zst" "%s/"\n' "$BUCKET" "$folder" $((4294967295-s)) "$s" "$dir"
  done > "$list"
  for t in 1 2 3; do echo "[$(date +%F\ %T)] s5cmd $f0..$f1 (t$t)"; "$S5CMD" --log error --no-sign-request run "$list" && break; sleep 15; done
  rm -f "$list"
  for (( slo=lo; slo<=hi; slo+=P )); do                            # 16k RUN sub-windows
    shi=$(( slo+P-1 )); [ "$shi" -gt "$hi" ] && shi=$hi
    [ -s "$WM" ] && [ "$(cat "$WM")" -ge "$shi" ] && continue
    for t in 1 2 3; do
      echo "[$(date +%F\ %T)] run $slo..$shi (t$t)"
      "$BIN" --clickhouse-url "$CLICKHOUSE_URL" --temp-dir "$DATA" --keep-partitions --only "$ONLY" \
        run --reindex --start "$slo" --end "$shi" && { echo "$shi" > "$WM"; break; }
      sleep 30
    done
  done
  rm -rf "$dir"                                                    # after all sub-windows
done
echo "[$(date +%F\ %T)] DONE $START..$END"
```

Prove the flag and idempotency on one small slice first — **this is also the
check that the targeted write touches only the three tables**:

```bash
# BOX
set -a; source ~/meta.env; set +a
"$BIN" -v --clickhouse-url "$CLICKHOUSE_URL" --temp-dir ~/bf-dbg \
  --only asset_transfers,transaction_memos,soroban_event_ops \
  run --reindex --start 50457424 --end 50457999
docker exec app-clickhouse-1 clickhouse-client -q "
  SELECT 'asset_transfers' t, count() c,
         uniqExact((ledger_sequence, application_order, op_index, event_pos_in_op)) k
  FROM asset_transfers WHERE ledger_sequence BETWEEN 50457424 AND 50457999
  UNION ALL SELECT 'transaction_memos', count(), uniqExact((ledger_sequence, application_order))
  FROM transaction_memos WHERE ledger_sequence BETWEEN 50457424 AND 50457999
  UNION ALL SELECT 'soroban_event_ops', count(), uniqExact((ledger_sequence, transaction_id, event_index))
  FROM soroban_event_ops WHERE ledger_sequence BETWEEN 50457424 AND 50457999"
# expected for ledger 50457424 alone (harness, 2026-09-05): 933 token events → 933 asset_transfers rows
# no ledgers marker must appear:
docker exec app-clickhouse-1 clickhouse-client -q \
  "SELECT count() FROM ledgers WHERE sequence BETWEEN 50457424 AND 50457999 AND closed_at > now() - INTERVAL 1 HOUR"
# re-run the same slice → k must stay identical (RMT idempotent); c may shrink toward k on merge.
```

Fan-out. `END` is `L₀` from step 4; worker count is decision 20's (runbook
§6.4 says start at ~6):

```bash
# BOX
set -a; source ~/meta.env; set +a
rm -rf ~/bf-540; mkdir -p ~/bf-540
S=50457424; E=<L0>; N=6
STEP=$(( (E-S)/N ))
for i in $(seq 0 $((N-1))); do
  Si=$(( S + i*STEP )); Ei=$(( i==N-1 ? E : S + (i+1)*STEP ))
  DATA=~/bf-540/w$i nohup ~/bf-loop-0540.sh $Si $Ei ~/bf-540/wm$i.txt > ~/bf-540/w$i.log 2>&1 &
done
jobs
```

Disk governance (runbook §7) applies with one difference: the targeted write
rewrites **no** other table, so the `OPTIMIZE` loop is only ever needed on the
three new tables, and `repair-tier1` is **not** owed.

Monitor from the laptop:

```bash
ssh sorban-prod 'df -h / | tail -1; pgrep -af bf-loop-0540 | wc -l; for f in ~/bf-540/wm*.txt; do echo "$f -> $(cat "$f")"; done'
```

## Gate 7a — coverage per partition (read-only, agent runs it via `chq`)

Rows in `asset_transfers` against the non-diagnostic token events
`soroban_events` already holds, per partition. They must agree except for the
rejects the decoder counted (`xdr_parser::asset_transfers` warnings in the
worker logs; expected ≈ 0 — the gate rejected 0 on 33 ledgers).

```sql
SELECT p,
       any(ev) AS token_events,
       any(at) AS transfers,
       any(ev) - any(at) AS diff
FROM (
    SELECT intDiv(ledger_sequence, 500000) AS p,
           count() AS ev, 0 AS at
    FROM soroban_events
    WHERE signature IN ('transfer','mint','burn','clawback') AND event_type = 1
    GROUP BY p
    UNION ALL
    SELECT intDiv(ledger_sequence, 500000) AS p, 0,
           uniqExact((ledger_sequence, application_order, op_index, event_pos_in_op))
    FROM asset_transfers
    GROUP BY p
)
GROUP BY p ORDER BY p
```

`soroban_events` carries unmerged duplicates too, so compare
`uniqExact((transaction_id, event_index))` there if `diff` is not ~0 before
reading anything into it. Gate 7b (archive re-decode diff) and 7c (T11) follow.

## Rollback at any point up to step 7

```sql
DROP TABLE asset_transfers; DROP TABLE transaction_memos; DROP TABLE soroban_event_ops;
```

Nothing else was written. If the indexer must be rolled back while the tables
exist, redeploy the previous Compute **first**, then drop.
