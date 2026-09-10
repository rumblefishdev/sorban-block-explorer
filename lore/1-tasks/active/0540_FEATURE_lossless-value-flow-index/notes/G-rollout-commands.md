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
  - date: 2026-09-07
    status: developing
    who: karolkow
    note: >
      Step 4 executed, and not as written: the deployed API reads
      `net_settled` in the transaction-list aggregate, so the drop-then-deploy
      order would have 500'd every transaction list. Replaced by
      defaults-then-deploy with the drops deferred past the backfills — no
      pause, no downtime, rollback stays free. L₀ = 64 317 019 recorded and
      the backfill bounds filled in.
  - date: 2026-09-06
    status: developing
    who: karolkow
    note: >
      Exact commands for the map owner's half of rollout step 3 (tables +
      snapshot), 5 (ship the binary) and 6 (the targeted backfill), plus the
      gate-7a coverage queries. Adapted from the Hetzner re-parse runbook; the
      only differences are the three tables and `--only`.
  - date: 2026-09-06
    status: developing
    who: karolkow
    note: >
      Revised after a devil's-advocate pass against 0488, 0310 and
      docs/backups.md: no local snapshot (a full copy is ~760 GiB against
      ~459 GiB free — it would reproduce 0488), scratch on /srv/bf-scratch
      with 3 workers and no -v, the deploy window spelled out as pause →
      ALTER → deploy → container recycle, tables created from the laptop, and
      the binary rebuilt from the merged commit.
---

# Rollout commands (task 0540)

Follows `docs/runbooks/backfill_derived_table_reparse_hetzner.md`, flavour B
(from-S3 re-parse), corrected by what task 0488 learned the hard way. Every
command below is the **map owner's**; nothing here is run by an agent. Order:
1 (merge) → 2 (checks) → 3 (tables) → 4 (deploy window) → 5 (binary) → 6
(backfill) → 7 (gates).

## Step 1 — merge first, do NOT tag

The branch is reviewed and merged to `develop`, then merged `develop →
master` — **without a release tag**. In this repo a tag IS the deploy
(`docs/deployment.md`: pushing `production-…` runs `cdk deploy` of the Compute
stack), and the new indexer must not reach production before the three tables
exist (step 3) and `net_settled` is dropped (step 4b) — "deploy first, ALTER
whenever" is the 0310 outage. The tag is pushed inside the deploy window, as
step 4c. Nothing below happens from a feature branch. The backfill binary is
rebuilt from **the commit that was deployed** (step 5) — the overlap around L₀
is only safe because both writers are the same code.

## Step 2 — before anything (the 0488 checklist)

Task 0488 (2026-08-13): eight backfill workers with scratch on `/` plus a
351 GB `tmux -v` log filled the only filesystem; ClickHouse refused writes and
live ingestion stopped for 10 hours, while the watchdog wrote alerts to a file
nobody read. What exists today against a repeat: the scratch image
`/srv/bf-scratch` (89 GB loopback, ENOSPC inside it never touches `/`) and
per-ledger file deletion. What does NOT exist: disk/lag alarms, a runner
budget. So the human is the alarm.

```bash
# BOX
df -h / | tail -1                              # do not start under ~300 GB free
mount | grep bf-scratch                        # /srv/bf-scratch MUST be mounted (the box had a pending reboot;
                                               # the fstab line may not have landed) — if absent, remount per 0488 step 1
df -h /srv/bf-scratch | tail -1                # ~89 GB, empty
ps -o args= -p "$(pgrep -x tmux | head -1)"    # MUST NOT contain -v
du -sh /home/deploy/*.log 2>/dev/null          # nothing growing
pgrep -af 'bf-loop|backfill-runner' | wc -l    # 0 = no other backfill (0419) running
```

New tables on `/` (all estimates, floor after merges; add 10–23% unmerged):
`asset_transfers` 41–49 GB, `transaction_memos` 1–3 GB, and
`soroban_event_ops` **~3.6 GB** — ~5.7 bn rows (measured 281 / 496 / 464
non-fee non-diagnostic events per ledger in three 10 001-ledger windows) at
**0.63 B/row**, measured 2026-09-07 on the local 39.5 M-row window with the
final DDL. The first DDL keyed the table by `transaction_id` and measured
5.07 B/row (~29 GB): 4.66 of those bytes were the id, a random hash that does
not compress; re-keying by `application_order` removed it. Total
**~46–56 GB** before the unmerged overhead. s5cmd scratch
(~13.6 GB per partition, two per worker) lives on `/srv/bf-scratch`, which
bounds the run to **3 workers** — that is the constraint, not a tuning choice.

## Step 3 — create the three tables (no snapshot)

**The DDL below is a copy for convenience; the source of truth is
`crates/db-clickhouse/schema/init.sql` at the deployed commit — diff the two
before running (`soroban_event_ops` was re-keyed on 2026-09-07 and this copy
lagged a day).**

Created **before** the indexer that writes them deploys (the driver validates
the row struct against `DESCRIBE`; a missing table fails every insert
client-side — task 0310). `CREATE TABLE IF NOT EXISTS`, so re-running is safe.
Verbatim from `crates/db-clickhouse/schema/init.sql` at commit `c62d12dd`.

From the **laptop**, in your own ClickHouse client session: the `dev_read`
user carries `CREATE`/`ALTER`/`DROP` despite its name (checked with
`SHOW GRANTS`), but `chq` runs on a read-only profile, so not through `chq`.
No SSH needed for this step. The `docker exec` form below is the box
equivalent if you prefer it.

**No pre-op snapshot.** `BACKUP DATABASE … TO Disk('backups')` is a full local
copy — the last one was 760 GiB — on the same volume, against ~459 GiB free:
it would fill the disk, which is exactly the 0488 failure. It would also
protect nothing: this change is additive (the targeted write touches no other
table — proven by `lp_amounts_targeted_write_e2e`), rollback is `DROP TABLE`
×3, and the weekly Borg cron runs regardless. If a pinned copy is wanted
anyway, use the off-box Borg archive (`docs/backups.md`, option B, 0 GB local).

```bash
# BOX form (or paste the SQL into your laptop client)
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
    application_order  Int16                   CODEC(ZSTD(3)),
    event_index        Int16                   CODEC(ZSTD(3)),
    op_index           Int16                   CODEC(ZSTD(3)),
    event_pos_in_op    Int16                   CODEC(ZSTD(3))
)
ENGINE = ReplacingMergeTree
PARTITION BY intDiv(ledger_sequence, 500000)
ORDER BY (ledger_sequence, application_order, event_index);
SQL

# confirm the DESCRIBE the driver will see
docker exec app-clickhouse-1 clickhouse-client -q "DESCRIBE asset_transfers" | head -20
```

## Step 4 — the deploy window — DONE 2026-09-07, and NOT as written below

**Executed as: defaults → deploy → drops deferred. No pause, no downtime.**
Tag `production-2026.09.07-1` on `098bef9d`. Keep the original text below as
the reasoning that led here, but do not follow its order again: it drops
`net_settled` before the deploy, and the deployed API reads that column in the
transaction-list aggregate, so that order answers 500 on every transaction list
for the length of the deploy.

What actually ran, from the laptop client, before the deploy and with the
indexer untouched:

```sql
ALTER TABLE operation_asset_appearances MODIFY COLUMN net_settled Nullable(Int128) DEFAULT NULL;
ALTER TABLE liquidity_pool_snapshots    MODIFY COLUMN tvl         Nullable(Decimal(38, 7)) DEFAULT NULL;
ALTER TABLE liquidity_pool_snapshots    MODIFY COLUMN volume      Nullable(Decimal(38, 7)) DEFAULT NULL;
ALTER TABLE liquidity_pool_snapshots    MODIFY COLUMN fee_revenue Nullable(Decimal(38, 7)) DEFAULT NULL;
```

The driver rejects a table column missing from the row struct **only when it
has no default** (`clickhouse-0.15.0`, `row_metadata.rs`,
`InsertMetadata::to_row`) — the INSERT names its columns explicitly, so a
defaulted extra column is never mentioned. A default-only `MODIFY COLUMN` is
metadata; verified afterwards that no mutation was created. Old and new writers
were therefore both valid against one schema, which removes the pause, the
recycle and the ordering constraint together. The DROPs are deferred until
after the backfills so a rollback to the previous binary stays free.

Confirm the defaults took, and that nothing is rewriting data:

```sql
SELECT table, name, default_kind FROM system.columns
WHERE database = 'default'
  AND ((table = 'operation_asset_appearances' AND name = 'net_settled')
    OR (table = 'liquidity_pool_snapshots' AND name IN ('tvl','volume','fee_revenue')));
SELECT max(create_time) FROM system.mutations;
```

**L₀ = 64 317 019** (measured after the deploy). Backfill range:
`50 457 424 .. 64 317 019`.

### Original plan, superseded — kept for its reasoning

The `clickhouse` 0.15 driver validates the row struct against `DESCRIBE` **in
both directions** and warm Lambda containers cache that `DESCRIBE`. So: old
indexer + dropped column fails; new indexer + column still there fails; new
indexer + missing tables fails. Task 0310 lost 9 minutes of ingest to "deploy
first, ALTER whenever". One window, in this order, nothing lost (the S3→SQS
notification keeps queuing while the Lambda is paused):

```bash
# LAPTOP — (a) pause the indexer (takes effect in under a minute)
aws lambda list-event-source-mappings \
  --function-name production-soroban-explorer-indexer \
  --query 'EventSourceMappings[].UUID' --output text
aws lambda update-event-source-mapping --uuid <uuid> --no-enabled
```

```sql
-- LAPTOP client or BOX — (b) drop the column the new struct no longer has (a2f8c5a7)
ALTER TABLE operation_asset_appearances DROP COLUMN net_settled
```

```bash
# LAPTOP — (c) deploy Compute per docs/deployment.md (diff first, then the
# Compute deploy target); CDK reconciles the ESM back to enabled

# LAPTOP — (d) recycle warm containers so no cached DESCRIBE survives (0310's fix)
aws lambda update-function-configuration \
  --function-name production-soroban-explorer-indexer \
  --description "0540 rollout $(date -u +%FT%TZ)"

# (e) confirm the ESM is enabled again and ledgers flow
aws lambda list-event-source-mappings --function-name production-soroban-explorer-indexer \
  --query 'EventSourceMappings[].State' --output text
```

Note the ledger the new indexer first writes — call it **L₀**:

```bash
docker exec app-clickhouse-1 clickhouse-client -q \
  "SELECT min(ledger_sequence) FROM asset_transfers"
```

`L₀` is the backfill's `END`. The overlap around it is safe: both writers are
the same commit, rows are byte-identical, the RMT collapses them.

## Step 5 — build and ship the binary, from the deployed commit

```bash
# LAPTOP — on the merged commit that step 4 deployed (git log -1 on master)
ulimit -n 65536
cargo zigbuild --release -p backfill-runner --bin backfill-runner \
  --target x86_64-unknown-linux-gnu.2.31
strings target/x86_64-unknown-linux-gnu/release/backfill-runner | grep -c 'GLIBC_2.3[2-9]'   # must print 0 (box is glibc 2.31)
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
before `run`. **No `-v` anywhere** (0488: verbose workers through a verbose
tmux wrote 385 GB/day):

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
    # Three failures must FAIL the run: without this the next sub-window's
    # success would move the watermark past the hole, and a resume would
    # skip it forever (the runbook this loop was copied from has the same gap).
    [ "$(cat "$WM" 2>/dev/null || echo 0)" -ge "$shi" ] || { echo "FAILED $slo..$shi after 3 tries"; exit 1; }
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
  UNION ALL SELECT 'soroban_event_ops', count(), uniqExact((ledger_sequence, application_order, event_index))
  FROM soroban_event_ops WHERE ledger_sequence BETWEEN 50457424 AND 50457999"
# expected for ledger 50457424 alone (harness, 2026-09-05): 933 token events → 933 asset_transfers rows
# no ledgers marker must appear:
docker exec app-clickhouse-1 clickhouse-client -q \
  "SELECT count() FROM ledgers WHERE sequence BETWEEN 50457424 AND 50457999 AND closed_at > now() - INTERVAL 1 HOUR"
# re-run the same slice → k must stay identical (RMT idempotent); c may shrink toward k on merge.
```

Fan-out. `END` is `L₀` from step 4 — **64 317 019**. **Scratch on `/srv/bf-scratch`, three
workers** — the 89 GB image holds ~2 partitions per worker and no more (0488
step 1; the runbook's "start at 6" predates the isolation). Logs go to `/`
but are bounded: no `-v`.

```bash
# BOX
set -a; source ~/meta.env; set +a
rm -rf ~/bf-540; mkdir -p ~/bf-540
S=50457424; E=64317019; N=3   # E = L0, measured after the 2026-09-07 deploy
STEP=$(( (E-S)/N ))
for i in $(seq 0 $((N-1))); do
  Si=$(( S + i*STEP )); Ei=$(( i==N-1 ? E : S + (i+1)*STEP ))
  DATA=/srv/bf-scratch/w$i nohup ~/bf-loop-0540.sh $Si $Ei ~/bf-540/wm$i.txt > ~/bf-540/w$i.log 2>&1 &
done
jobs
```

The targeted write rewrites **no** other table, so the runbook's `OPTIMIZE`
loop (§7) is only ever needed on the three new tables, and `repair-tier1` is
**not** owed.

**The human is the alarm** (0488's alarms are not built). Every ~30 minutes
while it runs, from the laptop:

```bash
ssh sorban-prod 'df -h / /srv/bf-scratch | tail -2; pgrep -af bf-loop-0540 | wc -l; \
  for f in ~/bf-540/wm*.txt; do echo "$f -> $(cat "$f")"; done; \
  du -sh ~/bf-540/*.log /home/deploy/*.log 2>/dev/null | sort -h | tail -3'
```

Hard rule: **free space on `/` below ~150 GB → `pkill -f bf-loop-0540` first,
ask questions after.** Ingestion lag (should stay minutes, not hours):

```sql
SELECT now() - max(closed_at) FROM ledgers
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
`uniqExact((transaction_id, event_index))` there (its own key) if `diff` is not ~0 before
reading anything into it. Gate 7b (archive re-decode diff) and 7c (T11) follow.

## Rollback at any point up to step 7

```sql
DROP TABLE asset_transfers; DROP TABLE transaction_memos; DROP TABLE soroban_event_ops;
```

Nothing else was written. If the indexer must be rolled back while the tables
exist, redeploy the previous Compute **first**, then drop. If step 4b already
ran, the previous binary's row struct still has `net_settled` and the driver
will reject every `operation_asset_appearances` insert against a table
without it — so **before** the redeploy:

```sql
ALTER TABLE operation_asset_appearances ADD COLUMN net_settled Nullable(Int128);
```
