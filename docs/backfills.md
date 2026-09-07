# Backfill Guide

Single entry point for backfilling ClickHouse. **Read
[§ Correctness rules](#correctness-rules--read-before-you-run-anything) before
running anything** — most of the ways a backfill goes wrong are silent.

Deep-dive runbooks are linked, not duplicated.

---

## Which situation are you in?

| Situation                                                                 | What to run                                                                                                        |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| **Gap** — ledgers missing (after a restore, an outage, a stalled indexer) | `backfill-runner run --start <gap> --end <tip>` — `run` is a **gap-filler**, it skips ledgers already in `ledgers` |
| **New derived table** over history — the data exists only in XDR          | from-S3 re-parse: `run --reindex` (see [§ On the box](#path-a--directly-on-the-hetzner-box-current-default))       |
| **New derived table** computable from columns already in CH               | cheap in-DB `INSERT … SELECT` (no re-parse)                                                                        |
| **Bad data in place** (range already in `ledgers`)                        | `run --reindex` — a plain `run` would **no-op**                                                                    |
| Tier-1 columns wrong after any of the above                               | `repair-tier1` (**mandatory** — see below)                                                                         |

---

## Correctness rules — read before you run anything

### 1. Writes are idempotent by construction

Backfill and the live indexer share one parse path
(`backfill-runner/src/ingest.rs` → `indexer::handler::process::process_ledger`).
Three properties make replay safe:

- **Absolute state, never delta.** Rows are post-images read straight from the
  XDR `TransactionMeta` — no read-modify-write, no carried in-memory balance. A
  removed trustline emits an explicit `balance: 0` row, not a decrement.
  **There is no double-apply.**
- **Deterministic surrogate ids** (`db-clickhouse/src/persist/ids.rs`) — the file
  names "Parallel-writer safety" as an explicit design goal: K runners compute
  the same `id` for the same StrKey with no shared counter, no coordination.
- **Commit marker written last.** The `ledgers` row is opened strictly after the
  18 entity tables. A marker-less ledger is re-processed in full.

`insert_deduplicate = 0` — ReplacingMergeTree (RMT) is the dedup layer, not the
insert path.

**Engine inventory:** 25 of 28 tables are RMT. The 3 that are not are all
duplicate-proof, so they are **not** a hazard: `accounts_recent` and
`balance_aggregates` are refreshable-MV targets (full recompute + atomic
`EXCHANGE`, nothing writes to them directly), and `asset_sac` is
AggregatingMergeTree over `max` (`max(x,x) = x`). There is no SummingMergeTree
and no append-only counter anywhere.

### 2. Parallelism — K processes on disjoint ranges

There is **no `--workers` flag**. The runner is sequential per-ledger, one
partition at a time. Parallelism is external: run K processes with disjoint
`--start`/`--end`.

- **Safe:** concurrent inserts on the same table need no coordination — RMT
  dedups merge-side keyed by `ORDER BY`. Overlapping ranges are _tolerated_
  (re-running an overlap is a no-op); with identical code the rows are
  byte-identical anyway, and where they are not, the later insert wins. See
  rule 4.
- **K ≥ 8:** raise CH `max_concurrent_queries` from 100 to ~200 (each writer
  holds ~14 long-lived inserts). Performance, not correctness.
- **Empirically (2026-07, 24 cores):** more than **6 workers was no faster**, and
  going past 6 only worsened disk pressure.

> **Disjointness is necessary but NOT sufficient.** See rule 3.

### 3. The MIN-semantics trap — 12 Tier-1 columns corrupt silently

RMT keeps the **highest-version whole row**, so it **cannot express MIN
semantics**. Worker N stamps `first_seen_ledger` with the first ledger of _its_
range, with no visibility into earlier workers' ranges — so the surviving value
reflects the latest-touching worker, not the true minimum. **Twelve Tier-1
columns corrupt this way, silently.**

Scale, measured: the 0228 repair moved `first_seen_ledger` for **10.13M
accounts**.

→ **Budget a `repair-tier1` pass after every parallel or `--reindex` backfill.**
`repair-tier1` itself requires the indexer stopped (see the table below).

> **This is NOT only a backfill trap — it also happens on ordinary live
> ingest** (task 0531, measured 2026-09-01). The indexer sees only its current
> batch, so any later event for an entity carries no historic minimum and the
> RMT replace erases whatever was stored. No parallel run is required.
>
> - `nfts.minted_at_ledger` — 643 of 13 932 tokens were wrong, growing **~30 per
>   day**. Served correctly since task 0528, which reads the value from the
>   append-only `nft_ownership` instead of the stored column.
> - `accounts.first_seen_ledger` — **14 of 400 sampled rows diverge (3.5%)**, all
>   of them later than the true first appearance. Still wrong today, and it is
>   rendered on the account page and the account list.
> - `soroban_contracts.deployed_at_ledger` — **1 597 of 146 397 diverge (1.1%)**.
>
> So a clean `repair-tier1` after a backfill does **not** mean the Tier-1 columns
> stay correct: they start drifting again immediately. Treat the pass as
> point-in-time cleanup, not as a guarantee. Task 0531 replaces it with storage
> that carries MIN semantics natively, and retires this rule.

**Unless the run writes one table that has no such column.** A re-parse whose
only purpose is to populate a NEW derived table does not need to re-emit the
other twenty-odd — and if it does, it re-arms this trap for nothing. Task 0266
did this with a bespoke harness ("targeted write only — do NOT run the full
persist pipeline"); task 0279 turned it into a flag:

```bash
backfill-runner run --start <A> --end <B> --lp-amounts-only
```

It parses exactly as a normal run does and persists only
`lp_operation_amounts`, so **no Tier-1 column is touched and no `repair-tier1`
is owed**. The trade is that it writes no `ledgers` commit marker (the marker
means "fully ingested", which a targeted pass has not done), so resume cannot
read progress from the DB: on a crash, restart with a narrowed `--start`.
Re-running a range is harmless — the rows are deterministic and the RMT
collapses the duplicates.

Adding a second such mode is a one-line branch beside it in
`sink.rs::write_ledger`; the pattern generalises to any future
one-new-table re-parse.

Two nuances worth knowing:

- This is **not parallel-specific**. Per task 0232 the same columns also drift in
  **live ingest**, because the CH writer cannot afford the read-before-write the
  old PG writer used. Parallelism widens an existing defect rather than creating
  one.
- Backfill commits per **64k**-ledger unit while CH partitions are **500k** — so
  ~8 writers land in one CH partition. This mismatch is why partition-swap
  atomicity was rejected.

### 4. Version-less RMT keeps the LAST ROW INSERTED — so emit one row per key

15 of the RMT tables carry no version column. The rule that matters there is
about **one insert**, not about re-parsing.

**Re-parsing is safe.** A re-parse lands after the data it replaces, and
version-less RMT keeps the last row inserted, so the newer parse wins. Measured
on a ClickHouse 26.3 server (lore 0425), in the shapes that actually occur:

| Shape                                                                     | Result                                               |
| ------------------------------------------------------------------------- | ---------------------------------------------------- |
| 40 unmerged old parts, then a 4-way concurrent re-parse, read via `FINAL` | new value wins, no survivors                         |
| background merges only, never `OPTIMIZE`                                  | new value wins                                       |
| old data already `OPTIMIZE FINAL`-collapsed, then re-parsed               | new value wins                                       |
| partial re-parse (half the keys)                                          | re-parsed keys update, the rest keep their old value |

This also holds structurally: every version-less table is either **keyed by
ledger** — so a re-parse of ledger N only ever competes with its own earlier
parse of ledger N — or a **pure function of an immutable input**
(`wasm_interface_metadata` by `wasm_hash`; `assets`, whose mutable columns are
DEAD and now live in `balance_aggregates` / `asset_enrichment`).

**The real hazard is two rows for one key inside a single insert.** Then "last"
means whatever order the code emitted, which carries no meaning. That is
task 0356: the parser emitted the before- and after-image of a pool per op, so
one `(pool, ledger)` key got two rows with different reserves and ClickHouse kept
one at random — wrong data, not duplicates.

→ **A parse must emit at most one row per key per insert.** Get that right and
`run --reindex` needs no further ceremony. (Tables that DO carry a version — 11
of them, all keyed by an entity rather than a ledger — need it for a different
reason: a re-parse of old ledgers inserts old state last, and only the version
stops it from rolling current state backwards.)

### 5. A version-less RMT row replaces the WHOLE row — emit complete rows

A sparse/partial row does not patch a column; it **replaces the row**, silently
nulling or zeroing everything you left out. Recorded the hard way in 0266 for
`liquidity_pool_snapshots` reserves.

**Live example (task 0356):** the parser emitted both the `state` (before) and
`updated` (after) image per op, so one `(pool, ledger)` got two rows with
different reserves and version-less RMT picked one **at random** — wrong data,
not duplicates. The parser fix (`dedup_final_pool_snapshots`, "keep the last
image") **has landed** and applies to backfill too via the shared path — but the
lesson generalises: **any** version-less RMT table where a parse can emit more
than one row per key is exposed to the same silent corruption, and the bad rows
survive until someone rewrites them.

---

## Indexer: stop it, or not?

The dividing line is **`EXCHANGE TABLES`**. A subcommand that builds a staging
table and swaps it will **lose any live write** that lands between build and
swap.

| Must **STOP** the indexer (staging + `EXCHANGE TABLES`) | No stop needed (RMT, idempotent)                          |
| ------------------------------------------------------- | --------------------------------------------------------- |
| `contract-type-rebuild`, **`repair-tier1`**             | `run` (disjoint ranges), `balance-seed`, `nft-reclassify` |

**Grey zone:**

- **`bootstrap`** — computes its watermark **once** at start to win the RMT race.
  A concurrently-running indexer that advances `last_seen_ledger` past that stamp
  out-versions the bootstrap rows, leaving `seq=0` skeletons. Degraded, not
  corrupt; re-runnable.
- **`nft-reclassify`** — uses `ALTER TABLE … DELETE` (not `EXCHANGE`), so a live
  indexer re-inserts rows the DELETE just removed.

**How to stop it:** see
[`docs/deployment.md` § Backend Lambdas](deployment.md#backend-lambdas--compute-stack).
Short version — quick/temporary: disable the SQS event-source-mapping
(`aws lambda update-event-source-mapping --no-enabled`), **undone by the next
Compute deploy**. Durable: `indexerLambdaConcurrency: 0` in
`infra/envs/production.json` + redeploy Compute.

Either way **nothing is lost** — the S3→SNS notification is always wired
(not gated on concurrency), so a paused indexer still captures events durably in
the queue. Note prod runs `indexerLambdaConcurrency: 1` — a deliberate **single
writer**.

---

## Path A — directly on the Hetzner box (current default)

Build the binary on your laptop, ship it to the box, run it there against
`localhost:8123`. No transfer step, so no schema-drift or ATTACH concerns.

➡️ **Runbook: [`docs/runbooks/backfill_derived_table_reparse_hetzner.md`](runbooks/backfill_derived_table_reparse_hetzner.md)**
— covers both flavours end to end:

- **Flavour A — cheap in-DB backfill**, no re-parse (`arrayJoin` an existing
  column, RMT-dedup). Minutes.
- **Flavour B — from-S3 re-parse** (`s5cmd` pre-fetch + `run --reindex`), when
  the new grain exists only in the ledger XDR. ~1 TB of XDR streamed, ~a day,
  **disk-governed**.

Points worth knowing before you open it:

- The box has **no repo, no cargo, no `aws`/`s5cmd` by default** — the binary is
  cross-compiled on the laptop (zigbuild → glibc 2.31 x86_64) and `scp`'d.
- **Disk is the binding constraint** — root `/dev/md1` ≈ 1.8 TB; the 2026-07 run
  peaked at **92%**. Do not start under ~300 GB free. `--reindex` rewrites _every_
  table, so parts outrun merges — governed by TRUNCATE-ing system logs and
  `OPTIMIZE … PARTITION FINAL`.
- **The target table must be created by hand** — `init.sql` is fresh-install only
  and never re-runs against the live DB.
- **Take the pre-op snapshot first** and drop it **only after validation passes**
  — see [`docs/backups.md`](backups.md).

## Path B — backfill on a separate machine, then transfer to Hetzner

Used when the box itself cannot do the work (CPU/disk), historically with the
range split across machines. **Two transports — they are alternatives for
different situations, not competitors:**

|              | **Direct INSERT** (task 0266, June 2026)                                                  | **FREEZE + rsync + ATTACH PART** ([ADR 0045](../lore/2-adrs/0045_clickhouse-local-backfill-then-mirror-to-hetzner-via-freeze-rsync-attach.md), task 0228, May 2026) |
| ------------ | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Schema**   | **Tolerates drift** — maps columns by name                                                | **Requires byte-identical schema** — ATTACH rejects a structurally different part                                                                                   |
| **Best for** | incremental-on-live (backfill and live ingest are RMT-key-disjoint → **no pause needed**) | bulk / cold-build — moves immutable parts, no re-insert cost                                                                                                        |
| **Cost**     | serialize + zstd + ssh + INSERT (goes through the CH write path)                          | bandwidth/CPU-efficient                                                                                                                                             |
| **Status**   | the **default** since 0266                                                                | kept as the **bandwidth fallback**                                                                                                                                  |

**Decision rule: schema identical + bulk → ATTACH. Schema drifted, or
incremental-on-live → INSERT.**

### B1 — Direct INSERT (default)

Stream from the worker straight into prod over an ssh pipe:

```bash
# WORKER → PROD, one table, a bounded range
clickhouse-client -q "SELECT * FROM <table> WHERE ledger_sequence BETWEEN <a> AND <b> FORMAT Native" \
  | zstd -3 \
  | ssh sorban-prod "zstd -d | docker exec -i app-clickhouse-1 \
      clickhouse-client -q 'INSERT INTO <table> FORMAT Native'"
```

Why 0266 chose this over ADR 0045: prod had a scalar `pool_id` while the worker's
parts had only `pool_ids` — **ATTACH would have rejected the structurally
different part**, INSERT maps by name. The recorded conclusion: _"Transport
default flips to direct INSERT; ADR 0045 stays as bandwidth fallback."_

0266 also recorded that backfill and live ingest are **RMT-key-disjoint**, so no
ingestion pause was needed — _"the 0228 pattern was a cold-build design; this is
incremental-on-live, a different shape."_

### B2 — FREEZE + rsync + ATTACH PART (ADR 0045)

The cold-build path. Executed once (0228, validated GREEN: 980/980 Horizon
hash-set compare, 0.0000% mismatch).

```
worker:  backfill locally
         → scripts/merge-freeze-worker.sh      # FREEZE → /var/lib/clickhouse/shadow/<snap>/ (hardlinks)
         → rsync -avP --partial  <snap>/  hetzner:/var/lib/clickhouse/detached_inbox/<worker>/<snap>/
hetzner: → scripts/merge-attach-hetzner.sh     # mv → detached/ → ATTACH PART → OPTIMIZE FINAL → RELOAD DICT
         → backfill-runner repair-tier1        # mandatory (rule 3) — indexer STOPPED
worker:  → SYSTEM UNFREEZE WITH NAME '<snap>'
```

Gotchas, all recorded:

- **The rsync step has no script.** `merge-freeze-worker.sh` prints it as a manual
  instruction ("Next steps (NOT executed by this script)"). Automation stops on
  both sides of the wire.
- **Atomic-engine UUIDs differ per machine** — hence the `uuid_table_map.json`
  manifest, generated on the worker from `system.tables`; ADR 0045 calls this
  "easy to script but easy to fumble".
- **ATTACH in worker order** (m1 → m2 → m3) so the downstream repair pass has
  deterministic input.
- **Per-partition `OPTIMIZE … FINAL`** after each partition attaches (RMT collapse
  - straddle merge). Pass `optimize_throw_if_noop = 0` and a generous
    `max_execution_time` — the 60 s default is far too tight (0266: partitions
    100–126 took ~37 min).
- **Rollback:** before rsync → `UNFREEZE`. After ATTACH → `DETACH PART`. After
  `OPTIMIZE FINAL` → destructive; recovery means re-rsync from the worker's
  `/shadow/`, which is why workers keep `/shadow/` until the post-attach OPTIMIZE
  is confirmed.

> The operator runbook for this path (`merge-parallel-backfills.md`) was planned
> as task 0233 and **canceled as obsolete** in 2026-05 ("no future parallel
> backfill is planned"). The canceled task still carries the full section-by-section
> spec and is the best historical reference; the scripts' header links to that
> runbook are dangling.

---

## `backfill-runner` reference

**Subcommands:** `run`, `status`, `bootstrap`, `repair-tier1`,
`contract-type-rebuild`, `balance-seed`, `nft-reclassify`. Most one-shot ops
subcommands take `--dry-run`. No separate bins remain.

Seven spent one-shots were removed in lore 0425 — `wasm-upgrade-backfill` (0320),
`upgradeable-backfill` (0327), `nft-reparse` (0296), `soroban-token-flow-backfill`
(0383), `pool-ids-backfill` (0266) with its `scripts/0266/` wrappers,
`assets-id-backfill` (0331), and `metadata-backfill` (0304) — because the live
indexer now does each of them itself, verified on prod per command. Before writing a new one-shot, read the
authoring rule in
[`crates/backfill-runner/README.md`](../crates/backfill-runner/README.md#subcommands--the-rule-for-one-off-passes):
if the signal is not already in ClickHouse, the answer is `run --reindex`, not a
new binary.

**Flags — with the traps:**

| Flag                | Reality                                                                                                                                                                                                  |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--start` / `--end` | u32, inclusive. This is also how you parallelise (disjoint ranges).                                                                                                                                      |
| `--reindex`         | Bypasses the resume-skip so an already-ingested range is re-parsed. Without it, re-parsing history is a silent **0-row no-op** — `run` skips whatever is already in `ledgers`.                           |
| `--lp-amounts-only` | Persists **only** `lp_operation_amounts` (task 0279). Implies `--reindex`. Writes no `ledgers` marker, so resume is manual — narrow `--start`; re-running a range is a no-op. See the rule-3 note below. |
| `--keep-partitions` | **Debug only.** "Do not pass this for a real backfill — disk grows linearly."                                                                                                                            |
| `--target`          | **Does not exist.** Survives only in stale doc comments; PG was retired (0244), CH is the sole target.                                                                                                   |
| `--workers`         | **Does not exist.** Run K processes instead.                                                                                                                                                             |

**Config** (flag-or-env): `CLICKHOUSE_URL`, `CLICKHOUSE_USER`,
`CLICKHOUSE_PASSWORD`, `CLICKHOUSE_DATABASE`; `CLICKHOUSE_CERT` / `_KEY` / `_CA`
for mTLS via Caddy (all three or none — the CN maps to a CH user);
`SOROBAN_RPC_URL` (optional — unset ⇒ bootstrap skipped, accounts stay skeleton
rows); `BACKFILL_TEMP_DIR` (default `.temp/backfill-runner`).

---

## After any backfill

1. **`repair-tier1`** — mandatory after parallel or `--reindex` runs (rule 3).
   Stop the indexer first.
2. **Validate** — coverage gap-scan + a sample compare against raw ledger
   entries (`getLedgerEntries` XDR) or stellar.expert. NOT Horizon: it is
   legacy, and it synthesises fields the ledger does not carry.
   Re-run the same slice: the RMT-deduped count must stay identical.
3. **Only then** drop the pre-op snapshot ([`docs/backups.md`](backups.md)).

> Steps 1–2 are not bookkeeping. A re-parse that skips `repair-tier1` leaves the
> 12 Tier-1 columns wrong (rule 3), and the damage is invisible until someone
> reads `first_seen_ledger` — which is why "the write finished" is not the same
> as "the backfill is done".

## Checkpoint-snapshot passes (task 0463 / ADR 0055)

The SDF history archive publishes the **complete state of pubnet** at each
checkpoint as a bucket list (~4.5 GB gzipped, 21 files). This is a different
kind of source from everything above: backfills replay **changes** we already
hold, while the snapshot answers **"what does the network have that we do
not?"** — the only question a change-stream can never answer (78.85% of chain
history predates our ledger floor).

One subcommand, read-only except the seed's explicit `--execute`.
There are NO manual exports: each command reads our side straight from
ClickHouse through the same mTLS connection `--execute` inserts through, like
every other corrective command in this crate. (The research-phase probes
`snapshot-tally`/`snapshot-dedup`, the `snapshot-export-sql` helper and the
hand-exported-TSV transport were removed in the 2026-08-20 review;
the seed's dry-run IS the four-way comparison — a separate `snapshot-compare`
carried the same decode and the same verdict behind its own counting shell.)

| Subcommand                                      | What it does                                                                                                                                                                                                                                             | Writes                                                                          |
| ----------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- |
| `snapshot-seed [--artifacts <dir>] [--execute]` | build ALL corrections (missing holdings, closure stamps, ghost zeroing, signers, dimension stubs); dry-run by default; always decodes the freshest checkpoint, writing into `<artifacts>/<checkpoint_ledger>/` (default root `.artifacts/snapshot-seed`) | `balances`, `account_entry_state`, `assets`, `accounts` — only with `--execute` |

**The decision table.** Every one of our rows falls into exactly one verdict,
and the verdict alone decides what (if anything) is written. Read the report's
buckets against this:

| our row          | network at the checkpoint | relation                  | verdict                       | what is written                                             |
| ---------------- | ------------------------- | ------------------------- | ----------------------------- | ----------------------------------------------------------- |
| —                | live                      |                           | `missing`                     | live row @ the entry's own ledger                           |
| open             | absent                    | ours ≥ checkpoint         | `newer than checkpoint`       | nothing — the snapshot is the stale side                    |
| open, amount 0   | absent                    | ours < checkpoint         | `closure`                     | amount 0, closed @ checkpoint                               |
| open, amount > 0 | absent                    | ours < checkpoint         | **`GHOST`**                   | amount 0, closed @ checkpoint, **+ a line in `ghosts.tsv`** |
| closed           | absent                    |                           | `already closed`              | nothing                                                     |
| closed           | live                      | network newer             | **`CLOSED BUT LIVE`**         | live row @ the entry's ledger — re-opened                   |
| closed           | live                      | network not newer         | **`CLOSED vs LIVE conflict`** | nothing — defect signal, see below                          |
| open             | live                      | amounts equal, ours older | `stale`                       | nothing                                                     |
| open             | live                      | amounts equal             | `agree`                       | nothing — the positive control                              |
| open             | live                      | differ, network newer     | `heal`                        | the network's amount @ its ledger                           |
| open             | live                      | differ, ours newer        | `divergent ours newer`        | nothing — the live parser saw more                          |
| open             | live                      | differ, SAME ledger       | **`divergent SAME ledger`**   | nothing — defect signal                                     |

**Version discipline:** a live fact versions on the entry's own
`lastModifiedLedgerSeq`; an absence fact (closure, ghost) on the run's
checkpoint ledger, meaning "true at or before". Never a synthetic stamp. The
`≥ checkpoint` guard is deliberate, not an off-by-one: a checkpoint-versioned
correction written against a row already AT that ledger would be a
same-version ReplacingMergeTree tie, resolved arbitrarily — the exact
nondeterminism this tool exists to remove.

The `missing` split is reported for BOTH populations — trustlines and native
accounts — each with its own below/above-floor counts, 2M-ledger bands and
sample dump. An above-floor missing ACCOUNT is the stronger signal of the two:
native is the population the RPC bootstrap already seeded, so dormancy
explains it less well than it explains a dormant trustline.

Both defect signals get their own sample dump (`divergent_same_ledger.tsv`,
`closed_but_live_conflict.tsv`) — they write nothing, so the dump is the only
way to look at one.

**The two defect signals never auto-heal.** `divergent SAME ledger` means one
of two parsers misread that ledger; `CLOSED vs LIVE conflict` means something
closed a holding the network still has, at a ledger no honest version can
supersede. Both are reported and left alone: adopting a side, or inventing a
version, would bury the only evidence. Expect both at zero on the first run —
`CLOSED vs LIVE conflict` is structurally unreachable until something has
stamped a closure, so it is the alarm for the reconciliation runs below, where
the closures under test are the seed's own previous output or the live
writer's.

**Same-version ties are superseded, with one known exception.** A re-parse can
write two rows for one key at one ledger; ReplacingMergeTree then resolves them
arbitrarily and so does the API's own `argMax`. The seed supersedes both sides
by writing at the checkpoint version, which is strictly higher. That works for
every tie measured so far (1,238,583 keys, all merged accounts, all carrying
`closed_at_ledger = 0` on both sides — they predate the column), because such a
key lands in `closure`/`ghost` whichever side wins.

The exception, written down rather than implied: a tie between
`{amount: X, closed_at: 0}` and `{amount: 0, closed_at: L}` resolves to one
real row, and if that row is the second it reads as an ordinary
`already marked closed`, writes nothing, and the tie survives. Detecting it
needs the read to carry the tie itself — distinct contents at the winning
version — which is a second aggregation level over the whole table. Not built:
no such tie exists yet, because the process that generated the known ones
stopped ~2 months before the lifecycle writer made its first stamp. Re-run the
0503 tie query after any re-parse that touches stamped ledgers.

**`--execute` needs a write-capable ClickHouse identity.** The laptop mTLS cert
maps to user `dev_read`, whose profile sets `readonly = 1` and
`max_execution_time = 30` — an INSERT is refused on the readonly setting before
grants are consulted, and the row volume would exceed the execution ceiling
regardless. The dry-run is unaffected: it only reads, in bounded slices. Confirm
the identity before the run, not at the prompt (`SELECT currentUser()` and
`system.settings` answer it read-only; never test a write by writing).

### Running it — the exact shape

The mTLS bundle is three flags, all-or-nothing; a partial set panics. They can
also come from `CLICKHOUSE_CERT` / `_KEY` / `_CA`, but a read cert usually
already occupies those names, so pass the write bundle explicitly — a CLI value
beats the env var, so nothing has to be unset. `--artifacts` belongs to the
SUBCOMMAND, after `snapshot-seed`, not before it.

```bash
backfill-runner \
  --clickhouse-url https://<ch-host> \
  --ch-cert <cert.pem> --ch-key <key.pem> --ch-ca infra-hetzner/ca/ca.crt \
  snapshot-seed --artifacts <dir> [--execute]
```

**Step 0 — confirm the identity with a READ, never a trial write:**

```sql
SELECT currentUser(), getSetting('readonly'), getSetting('max_execution_time')
```

`--execute` now refuses early on a read-only identity rather than discovering it
at the first INSERT, but running the query yourself still costs 2 seconds and
tells you which user you are before anything downloads.

Keep a write cert OUT of any directory a read-only helper globs (the `chq`
wrapper takes the first `*.crt` and the first `*.key` INDEPENDENTLY, so a second
pair there either silently promotes that helper to admin or pairs mismatched
halves).

**The seed's ordering contract (do not reorder):**

1. Deploy the lifecycle writer (the indexer that stamps `closed_at_ledger`)
   FIRST.
2. Run `snapshot-seed --execute` against a checkpoint taken
   AFTER the deploy. Reversed, every removal between checkpoint and deploy is
   written by the old writer as a plain zero with a HIGHER version than the
   seed's closure — the ghost resurrects. The tool reads our rows itself at
   run time, so input freshness — measured as the dominant lever on correction
   volume — is no longer an operator concern. Churn between the dry-run read
   and the execute read is absorbed by the `>= checkpoint` guard (such rows
   are classified newer-than-checkpoint and left alone), so the dry-run's
   `summary.txt` is a close estimate of the execute's counts, never a
   contradiction of them.

**`--execute` never decodes the checkpoint the dry-run reviewed — expect that,
and read `summary.txt` accordingly.** Checkpoints publish every 64 ledgers
(~5 minutes) and a full pass takes about as long again — measured 317 s for a
dry-run and 637 s with the inserts, dominated by the archive download — so what
separates two runs is a whole pass plus the time an operator spends reading
`summary.txt`. The snapshot has therefore always advanced by the time the
second run starts, though by one checkpoint interval rather than several. Two
consequences, in opposite directions:

- Our own churn is harmless: the `>= checkpoint` guard files it as
  newer-than-checkpoint and leaves it alone.
- The network's is not purely cosmetic: holdings created between the two
  checkpoints are `missing` in the execute run and get INSERTED without having
  been in the reviewed summary. They are real live holdings — a fresher
  snapshot is a better input, not a riskier one — but the reviewed document
  bounds the run's counts, it does not enumerate its rows.

A `--pinned-manifest` flag existed for exactly this and was removed (its
original job was keeping a frozen manual export consistent with the snapshot,
and manual exports are gone). What replaces it is verification of the OUTCOME
rather than of the input: the coverage measurement and 200-account probe below
compare the finished table against the network. `manifest.json` records which
checkpoint each run actually used.

**No indexer stop is needed.** Every seeded row versions on a real ledger:
live data on the entry's own `lastModifiedLedgerSeq`, closures on the run's
checkpoint ledger (semantics: "closed at or before"). ReplacingMergeTree keeps
the higher version, so the live writer's newer rows always win regardless of
load order.

### Auditing a run that has written — the order that makes it mean something

Do these AFTER `--execute`, and capture the aggregates BEFORE it (they cannot be
recovered afterwards). Every number here was produced this way on 2026-08-26.

1. **Reproduce the run's own arithmetic.** The verdict buckets must sum to the
   rows it says it folded, and the writing verdicts must sum to the correction
   count. If either fails, a row was mis-bucketed and the rest of the audit is
   meaningless.
2. **Count the cohorts in ClickHouse.** Rows at `last_updated_ledger =
<checkpoint>` are the closure/ghost cohort (expect `amount = 0` and
   `closed_at = checkpoint`); rows below the ledger floor with `closed_at = 0`
   are the inserted live holdings.
3. **Sample from the DATABASE, not from the dumps.** The dumps are what the run
   _intended_; the table is what it _did_. Resolve identities through our own
   dimension tables and verify against `getLedgerEntries`, gated on
   `lastModifiedLedgerSeq` — equal licenses an amount claim, above is churn,
   below is a defect. Spread the sample across ERAS and across both asset-code
   widths: `AlphaNum4` and `AlphaNum12` are different XDR branches, and a sample
   of only short codes proves nothing about long ones.
4. **Check ingest and disk**, because a bulk insert is also the window where the
   ClickHouse driver can reject the live writer's rows: our head must equal the
   chain head, and the live writer must still be stamping.
5. **Compare against an external indexer** — and check what its column MEANS
   before believing the delta. A count labelled "trustlines" may or may not
   include contract holders; verify by computing both of ours and seeing which
   one tracks. Exact equality on one asset is luck, not evidence; agreement
   within a fraction of a percent across several is.
6. **Reconcile supply against the ledger header**, not against a remembered
   figure. `total_coins` and `fee_pool` both live in the signed header
   (`getLedgers` → `headerXdr`, past the 32-byte history-entry hash). The
   account sum must fall BELOW `total_coins` by the fee pool plus everything
   held outside accounts — AMM reserves, claimable balances, contract-held.
   Two traps, both hit in practice: measure the AMM term in the SAME pass (a
   two-day-old figure drifted 381k XLM, which was 40% of the residual it fed),
   and say whether your sum includes contract-held balances — on pubnet that is
   ~920M XLM, so the two readings of "our native sum" differ enormously.
7. **Re-run the dry-run.** It writes nothing, costs one pass, and is the only
   real test of idempotency: `missing` and `closure` should collapse to churn,
   `already closed` and `agree` should absorb them. A bucket that fails to move
   locates the defect exactly.

**After the seed (standing requirement, not advice):** measure the coverage
achieved for trustlines and accounts separately, AND cross-check a sample
against the RPC route regardless of the result. The 200-account probe from
task 0463's notes is the repeatable check: zero accounts where the chain holds
more live zero trustlines than we do, and zero where we show more than the
chain.

**Provenance:** the run writes `manifest.json` (checkpoint ledger + the 21
bucket hashes) into `<artifacts>/<checkpoint_ledger>/`. The archive is
content-addressed, so that manifest alone IDENTIFIES the snapshot a run
decoded — the planned LP merge (ADR 0056) reads it to know which checkpoint
this seed used. `ghosts.tsv` records every positive-amount
row the seed zeroed.

## After any historical re-parse: reconcile against the snapshot (MANDATORY)

A re-parse of already-ingested ledgers with CHANGED writer code writes rows at
the SAME ReplacingMergeTree versions the old code used. Where the new code
emits different content, the table holds two rows at one version and the merge
picks a winner **arbitrarily** — and `argMax` reads flip the same coin. This is
not hypothetical: the 2026-06-23 merge-tombstone fix plus a re-parse of
54M–63.04M left **1,238,583** such keys in `balances`, every one a merged
account randomly showing 0 or its stale pre-merge balance.

There is no in-schema defence. A "later insert wins" tiebreaker would make a
REGRESSED re-parse deterministically overwrite good data — worse than a
detectable tie. The arbiter is the network:

1. After the re-parse, run the standing tie query (task 0503 carries it per
   table): keys with more than one distinct content at one version.
2. Run `snapshot-seed` against a fresh checkpoint — dry-run, review
   `summary.txt`, then `--execute`. Between-runs ties surface as
   `divergent SAME ledger` (live entities) and as closure/ghost corrections
   (dead ones). Dead-entity ties are repaired outright at the checkpoint
   version, which supersedes both sides; same-ledger divergences on LIVE
   entities are reported for a human call — one of two parser versions is
   wrong, and auto-adopting either would bury the evidence.

This is the same tooling as the one-off 0463 seed; the seed is one-off, the
reconciliation is not.

## Soroban-AMM pool passes (task 0374)

Run only AFTER the 0374 DDL + indexer deploy (see the deploy-order gotcha in
[deployment.md](./deployment.md) — reversing the order is the 0310 outage
class). Three catch-ups, then one closure check:

1. **Pool registry** — one-off generator, deliberately not in the tree
   (a one-off is not a maintained surface). Full workflow — restore, harvest,
   generate, insert:

   ```bash
   # restore the generator source verbatim
   git show 082ee364:crates/db-clickhouse/src/bin/gen_pool_registry_backfill.rs \
     > crates/db-clickhouse/src/bin/gen_pool_registry_backfill.rs
   # harvest the corpus AFTER the live writer is deployed (step-4 ordering
   # rule: the corpus must cover everything up to the writer's start)
   chq "SELECT ledger_sequence, event_index, topics_xdr, data_xdr
        FROM soroban_events WHERE signature='add_pool'
        ORDER BY ledger_sequence, event_index
        FORMAT JSONEachRow" > /tmp/add_pool_corpus.jsonl
   # generate the INSERT, then run it via chq / clickhouse-client
   cargo run -p db-clickhouse --bin gen_pool_registry_backfill \
     /tmp/add_pool_corpus.jsonl > /tmp/pool_registry_backfill.sql
   ```

   Two adjustments the resurrected source needs before running (it predates
   later schema decisions):

   - drop `share_token_id` from its INSERT column list — the column is
     removed from `liquidity_pools` by the 0374 DDL;
   - make it REFUSE duplicate registrations (one GROUP BY on the pool
     address): the generator cannot perform the live writer's router
     corroboration (instance storage is not in `soroban_events`), so a
     forged duplicate `add_pool` naming an already-registered pool would
     beat the genuine row on RMT merge. Zero duplicates measured
     2026-09-01; this guards the window between that measurement and the
     writer's start.

   Emits `liquidity_pools` rows (pool_kind=1); idempotent, safe to re-run.

2. **Reserve history + instance state** — historical re-parse of the soroban
   window with the new indexer (`backfill-runner run` over the range),
   filling `pool_state_changes` + `pool_instance_state` from ledger state.
   `pool_instance_state.plane_id` is load-bearing: reserve reads keep only
   rows whose plane matches what the pool itself declares, so a pool with no
   instance row shows no reserves until this pass covers it.
3. **Classic `legs` backfill** — one-shot pass reading the legacy pair
   columns and emitting rows with `legs` filled, versioned on each row's own
   `last_updated_ledger` (task 0374 legs-migration step 2).

**Window-closure check (mandatory)** — proves no registration slipped between
the backfill's snapshot and the live writer's start. A cardinality diff is
blind to substitution (one missing + one extra = 0), so the check reconciles
the canonical pool-id SETS in both directions — a pool's registry `pool_id`
is the 32-byte payload of its `C...` address, which ClickHouse can derive
(`base32Decode`, strkey layout: 1 version byte + 32 payload + 2 checksum):

```sql
-- registered on chain but missing from the registry — MUST return 0 rows
SELECT DISTINCT JSONExtractString(data_xdr,'value',1,'value') AS missing_pool
FROM soroban_events WHERE signature = 'add_pool'
  AND toFixedString(substring(base32Decode(
        JSONExtractString(data_xdr,'value',1,'value')), 2, 32), 32)
      NOT IN (SELECT pool_id FROM liquidity_pools WHERE pool_kind = 1);

-- in the registry but never registered on chain — MUST return 0 rows
SELECT hex(pool_id) AS extra_pool
FROM liquidity_pools WHERE pool_kind = 1
  AND pool_id NOT IN (
    SELECT toFixedString(substring(base32Decode(
             JSONExtractString(data_xdr,'value',1,'value')), 2, 32), 32)
    FROM soroban_events WHERE signature = 'add_pool');
```

Any `missing_pool` → re-run the generator (idempotent) and re-check. Any
`extra_pool` → investigate before proceeding (a row nothing on chain
registered should not exist).

## Soroswap pool passes (task 0518)

Run only AFTER the 0518 indexer deploy (writer-first, same reasoning as the
0374 passes). Everything is derivable in-DB — no S3 re-parse — but BOTH
passes are small one-off Rust generators, not pure SQL: the surrogate ids
are `cityhash_102_128` (NOT ClickHouse's `cityHash64`), so SQL cannot
compute `pool_id`/`plane_id`/leg surrogates.

1. **Pair registry** — harvest the `new_pair` corpus (the exact chq command
   sits in `crates/xdr-parser/tests/pair_factory_real_corpus.rs` module docs),
   run it through a one-off generator built on `parse_new_pair` +
   `factory_pair_registry_row` (mirror `gen_pool_registry_backfill`, task
   0374). 235 pairs measured 2026-09-02; idempotent.
   **MANDATORY corroboration leg (review #447):** this pass is events-only
   — instance storage is not in `soroban_events` — so without it the
   backfill accepts exactly what the live writer's gate exists to refuse
   (a forged `new_pair` emitted after the 2026-09-02 measurement would
   land permanently, and its own gapless 1..k counter passes the closure
   check). The generator must therefore RPC-read every pair's instance
   (`getLedgerEntries`, ~235 reads): DataKey 4 must equal the corpus
   emitter and the instance must exist — the same cross-check the stage
   e2e performs offline. A pair failing it is refused loudly, never
   written.
   **Closure check, three layers:** per factory the vendor's own monotone
   counter — `max(new_pairs_length) == count()`; the base32Decode set
   reconciliation from the 0374 section (`pool_kind = 1` both times); and
   one EXTERNAL anchor (the first two compare us with ourselves and are
   blind to tail truncation — max and count shrink together): RPC each
   LIVING factory's own `all_pairs_length` and require our per-factory max
   to equal it. A fourth layer is free while both projects share one
   ClickHouse: set-compare against the sibling registry
   (`SELECT contract_id FROM prices.pool_registry WHERE venue='soroswap'`)
   — an INDEPENDENTLY-SOURCED set (vendor API seed); expect theirs ⊆ ours
   and INVESTIGATE any only-theirs address (this exact check caught the
   config family's dead early factories on 2026-09-05). Their seed is a
   snapshot, so only-ours entries are normal (stale seed). Guard every chq harvest with a `DB::Exception` grep on the
   output — chq exits 0 even on a server error, so a truncated corpus
   otherwise self-certifies.
2. **Reserve history** — `sync` events carry ABSOLUTE reserves and the
   local e2e proved them value-identical to instance state on every one of
   1,563 (pair, ledger) keys, both directions — so history comes from
   `sync` and the live writer's state rows continue seamlessly (task 0518
   decision 63). One-off Rust pass over the harvested sync corpus
   (per-partition chq slices, same quota rule as the 0517 backfill),
   emitting `pool_state_changes` rows with `plane_id = the pair's own
surrogate`. Idempotent under the RMT key.
   **Check:** re-run the bidirectional sync↔rows comparison per partition —
   compared == equal, both remainders zero.
3. **Declarations (`pool_instance_state`) — pass 1 emits them too**
   (review #447 amendment; the previous "no history pass needed" wording
   contradicted the ADR 0058 read rule quoted in the 0374 section: reserve
   reads keep only rows whose plane matches the pool's own declaration, so
   a DORMANT pair with no declaration row — 21 pairs of the three dead
   factories alone never trade again — would have its ENTIRE sync-derived
   reserve history invisible forever). The rows are free from the same
   corpus: `plane_id = share_token_id =` the pair's own surrogate,
   `derived_at_ledger =` the registration ledger. The live writer then
   refreshes active pairs on their next activity. The one remaining gap:
   `total_shares` of dormant pairs stays 0 until touched — the pass-1 RPC
   leg already reads each instance, so record the live `TotalSupply` from
   the same response instead of leaving the false zero.

## Config-factory (Phoenix-family) pool passes (task 0518, third adapter)

Run only AFTER the adapter deploy (writer-first, as above). This family is
the one whose history CANNOT come from events alone: its events carry
per-field amounts, never absolute reserves (no `sync` analogue), and the
registration event is a bare pool address — legs/fee/share token live only
in the pool's own `CONFIG` ledger entry. Both passes therefore read RAW
LEDGERS — but only a targeted, harvested list, fetched over public HTTPS
(the `aws-public-blockchain` bucket, unsigned; recipe in
`crates/db-clickhouse/tests/pair_factory_stage_real_e2e.rs` docs), never a
full re-parse.

1. **Pool registry + declarations** — exactly the **14 registration
   ledgers** (the full population, listed in
   `crates/db-clickhouse/tests/config_pool_stage_real_e2e.rs`; harvest
   query in `crates/xdr-parser/tests/config_pool_real_corpus.rs`). One-off
   generator driving `extract_pool_family_writes` + the STAGING gate
   (never the row builder directly — the two-stage registration gate,
   membership list + conflict, must hold for the backfill exactly as it
   does live): registry rows AND the `pool_instance_state` declarations
   (share token) come from the same 14 files.
   **Closure check:** live `query_pools()` on the factory must be a
   **SUBSET of ours — never set-equality**: the factory's vector is
   mutable, and one real pool (`CAZ6W4WH…`, 25,873 events traded to
   63.77M) is already delisted from it while its history stands. A
   registry seeded from `query_pools()` alone silently loses that pool.
   The ⊆ check alone is tail-blind (a pool both delisted AND missing from
   a truncated harvest is invisible to it), so pin the expected count too:
   **20 registrations across 6 factory deployments as of 2026-09-05** —
   the harvest must reproduce at least that many, and any new ones must be
   newer than ledger 64,030,567. (The count was 14 for one day: the first
   harvest was scoped to the documented factory; the cross-check against
   the sibling project's independently-seeded `prices.pool_registry`
   surfaced the five dead early deployments' 6 pools — proof the external
   anchor earns its place. The full-history shape-wide sweep also hit the
   hourly read quota mid-run and truncated SILENTLY, which is exactly why
   every closure layer here demands the `DB::Exception` grep AND an
   external anchor.) Guard the chq harvest with a `DB::Exception` grep
   (chq exits 0 on server errors).
2. **Reserve history** — one-off Rust pass over the **harvested activity
   ledgers**: `SELECT DISTINCT ledger_sequence FROM soroban_events WHERE
contract_id IN (the 14 pool surrogates)` — 195,637 ledgers / 2.04M
   events measured 2026-09-03 (≈40 GB of per-ledger `.xdr.zst` over
   HTTPS). Fetch each, run `extract_config_pools`, emit
   `pool_state_changes` rows (`plane_id` = the pool's own surrogate).
   Idempotent under the RMT key. Reserve co-occurrence (both keys per tx)
   is measured, era-proof — a half-pair in the output is a bug, not data.
   **Check:** per-pool spot comparison against raw creation values plus
   the live writer's overlap window — same rule as the pair family's
   sync↔state anti-test, with raw ledgers as the second source.
3. **`total_shares` caveat (family-specific, recorded in the task):** the
   declaration row's value is STRUCTURALLY 0 for this family (the supply
   key never co-occurs with a post-creation CONFIG write) — the live
   supply is the share token's own tracked supply (separate SEP-41
   contract on the generic token pipeline). Do not "fix" it with a
   per-op writer: `pool_instance_state` is RMT whole-row on `pool_id`, and
   a config-less row would clobber `share_token_id` to 0.
4. **Standing live cross-check (review #447):** this is the ONE family
   with no event oracle (the pair family has `sync`, the router family the
   deposit⇄mint detector), so a pool-WASM upgrade re-purposing the u32
   keys would corrupt reserves silently. Two cheap defenses, per release
   or on a cadence: (a) RPC-read every pool `query_pools()` still lists
   (13 today) and compare reserves against our latest `pool_state_changes`
   row; (b) watch `executable_update` events (already ingested, task 0320)
   on the registered pools — an upgrade of a registered pool warrants
   re-running (a) immediately.

## Event-name backfill (task 0517) — in-DB, per partition

Fills `soroban_events.signature` for the ~3.8M historical rows whose name
lives under a non-Symbol first topic (the Soroswap/DeFindex/Blend label
convention and the Phoenix plain-&str convention). Pure `INSERT … SELECT`
over `topics_xdr` — flavour A, no re-parse, no S3. The SQL mirrors
`extract_event_signature` (stage.rs) exactly; version-less RMT keeps the
last insert per key (rule 4), so re-running a slice is harmless.

**Run it per partition** — a bare `WHERE signature IS NULL` scans all 10G+
rows in one query, which blows the hourly read quota; the partition key is
`intDiv(ledger_sequence, 500000)` and each partition is ~300-420M rows, so
one partition per query prunes cleanly (~4-5/hour under quota, or loop them
all as the box operator where no quota applies). Partition ids:
`SELECT DISTINCT partition FROM system.parts WHERE table='soroban_events' AND active`.

```sql
-- one slice; substitute {P} with a partition id and iterate
INSERT INTO soroban_events
SELECT
    contract_id, transaction_id, ledger_sequence, event_index, event_type,
    multiIf(
        JSONExtractString(topics_xdr,1,'type') = 'string'
          AND JSONExtractString(topics_xdr,2,'type') = 'sym'
          AND JSONExtractString(topics_xdr,2,'value') != '',
            JSONExtractString(topics_xdr,2,'value'),
        JSONExtractString(topics_xdr,1,'type') = 'string'
          AND JSONExtractString(topics_xdr,1,'value') != '',
            JSONExtractString(topics_xdr,1,'value'),
        CAST(NULL, 'Nullable(String)')
    ) AS signature,
    topics_xdr, data_xdr
FROM soroban_events
WHERE signature IS NULL
  AND intDiv(ledger_sequence, 500000) = {P}
  AND multiIf(
        JSONExtractString(topics_xdr,1,'type') = 'string'
          AND JSONExtractString(topics_xdr,2,'type') = 'sym'
          AND JSONExtractString(topics_xdr,2,'value') != '',
            JSONExtractString(topics_xdr,2,'value'),
        JSONExtractString(topics_xdr,1,'type') = 'string'
          AND JSONExtractString(topics_xdr,1,'value') != '',
            JSONExtractString(topics_xdr,1,'value'),
        CAST(NULL, 'Nullable(String)')
    ) IS NOT NULL
```

The trailing filter keeps still-unresolvable rows OUT of the insert — their
NULL row already exists, and re-inserting an identical NULL row would only
churn the merge. **Verification** (after all partitions):

```sql
-- the resolvable NULL population MUST be zero
SELECT count() FROM soroban_events
WHERE signature IS NULL
  AND JSONExtractString(topics_xdr,1,'type') = 'string'
  AND JSONExtractString(topics_xdr,1,'value') != ''
```

Verification criteria for the deployed result live in task 0374's
final-phase notes.

## Superseded — do not follow

- [`lore/3-wiki/backfill-execution-plan.md`](../lore/3-wiki/backfill-execution-plan.md)
  — the RDS `pg_restore` staging cutover. Carries a SUPERSEDED banner (2026-05-20).
  Historical only.
- **ADR 0040** is Postgres-era (BIGSERIAL surrogate-key remapping), superseded in
  practice by the deterministic-cityhash CH design but **not marked as such** — a
  trap for anyone reading it as backfill guidance.
