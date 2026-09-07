---
id: '0531'
title: 'DUPLICATE of 0497 / 0232 / 0421 — Tier-1 MIN columns corrupt on live ingest'
type: BUG
status: completed
related_adr: ['0040', '0044', '0045']
related_tasks: ['0497', '0232', '0421', '0528', '0529', '0322', '0228', '0310']
tags:
  [
    'clickhouse',
    'data-integrity',
    'api',
    'indexer',
    'ops',
    'duplicate',
    'superseded',
  ]
links: []
history:
  - date: '2026-09-01'
    status: backlog
    who: karolkow
    note: >
      Spawned from 0528. 0528 fixed ONE of six Tier-1 columns; the same
      MIN-under-RMT defect corrupts the other five, and prod sampling confirms
      it. Also corrects a premise carried since 0228: the corruption is NOT
      confined to cross-machine parallel backfill — it happens on ordinary live
      ingest, every time a later event lands in a later batch.
  - date: '2026-09-01'
    status: backlog
    who: karolkow
    note: >
      REDIRECTED after measurement, and 0529 folded in. The original plan
      ("apply 0528's read-time derivation to the other five") does NOT survive
      contact with `accounts`: a page-scoped derivation for 50 accounts reads
      116–181 M rows and 2.34–27.3 GiB per page (two independent prod slices).
      That is a hot path, so read-time derivation is out for the big fact
      tables and the fix has to change the STORAGE semantics instead. 0529
      (drop the nfts column) is absorbed here so the production `ALTER` is paid
      once, not twice.
---

# DUPLICATE of 0497 / 0232 / 0421 — do not work this task

> **Filed in error.** Three tasks already owned this defect when 0531 was
> created:
>
> - **0497** — the parent: "retire repair-tier1 — move every MIN-semantics copy
>   off RMT state tables". Its "Questions to answer" are precisely what
>   this session measured, and **the measurements have been moved there** under
>   "Findings — session of 2026-09-01/02".
> - **0232** — per-column live-mode drift mitigation for all six columns; already
>   defers to 0497 in premise.
> - **0421** — the `accounts` instance, and wider than the MIN problem: it covers
>   `sequence_number` and `home_domain` on the same whole-row write, with a
>   measured 61.71% of active accounts carrying `sequence_number = 0`, and it
>   already proposes the `AggregatingMergeTree` route with a schema sketch.
>
> **How the duplicate happened**, recorded so the failure is reusable: the
> pre-filing search was run, but its output was truncated with `head -8` and the
> visible rows were all archived files, so the conclusion "nothing tracks this"
> was drawn from a cut-off list. `0421` has `first_seen_ledger` in its filename
> and would have been the first hit without the truncation. A search whose output
> is truncated has not answered the question.
>
> **What this session did produce that was genuinely new** — and which now lives
> in 0497, not here: the corruption measured per column, the read-time and
> write-time cost measurements that sink two routes, the experiment proving a
> materialised view does not fire on `ATTACH PART`, and task **0528**, which
> shipped the first column off RMT and is therefore 0497's first landed instance.
>
> The body below is kept unedited for traceability only. Read 0497 instead.

## Summary

Six columns hold a MIN-over-history value inside a `ReplacingMergeTree` state
table. RMT keeps the row with the highest version and replaces it **whole**, so
any later event overwrites the historic minimum. This is not a backfill
artifact — it happens on ordinary live ingest.

0528 fixed `nfts.minted_at_ledger` by deriving it at read time. That worked
because `nft_ownership` is 23 k rows. **Measurement shows the same approach is
not affordable for `accounts`**, so this task fixes the storage semantics rather
than papering over them at read time, and then removes every column that becomes
dead — in a single production `ALTER` window.

## The premise this corrects

`repair_tier1.rs` and task 0228 attribute the corruption to cross-machine
parallel backfill: worker N stamps a first-seen value for its own ledger range
with no visibility into earlier ranges, and the post-merge RMT collapse keeps the
latest writer's value.

True but incomplete. A single indexer reproduces it on live ingest: it sees only
the current batch, so a transfer, burn or later appearance carries no historic
minimum and the RMT replace erases whatever was there. Under that reading
`repair-tier1` is not a post-backfill step — it is recurring cleanup for a defect
that never stops producing.

## Measured on prod (2026-09-01)

### Corruption

| Table               | Column                 | Finding                                                                                                                                                  |
| ------------------- | ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `nfts`              | `minted_at_ledger`     | 642 NULL + 1 stored `0` of 13 932 — **served correctly since 0528**                                                                                      |
| `accounts`          | `first_seen_ledger`    | **14 / 400 sampled (3.5%) diverge**, all LATER than the true first appearance → ~570 k of 16.17 M                                                        |
| `soroban_contracts` | `deployed_at_ledger`   | **1 597 / 146 397 diverge (1.1%)**, plus 3 NULL where the value is known                                                                                 |
| `nfts_pending`      | `minted_at_ledger`     | 4 / 277 NULL; **66 / 277 have no Mint row at all**                                                                                                       |
| `lp_positions`      | `first_deposit_ledger` | not measured — `operations_appearances` (6.85 B rows) sorts on `ledger_sequence`, so a per-pool check is itself a full scan. Same defect by construction |
| `soroban_contracts` | `deployer_id`          | shares the rebuild with `deployed_at_ledger`                                                                                                             |

`accounts.first_seen_ledger` is the exposed one: served on the account detail and
list responses, rendered in `AccountSummary.tsx` and `AccountsTable.tsx`.

Sampling note: the `accounts` figure is a 400-row deterministic slice
(`id % 4001 = 7`) joined to `transaction_participants` on its sort key. A census
is not affordable — that table holds 10.73 B rows. Treat 3.5% as an estimate with
a real sample behind it.

### Cost — why 0528's approach does not generalise

Page-scoped derivation, 50 accounts, `WHERE account_id IN (page)` against
`transaction_participants`, two independent prod slices:

| Slice            | Rows read   | Bytes read | Duration |
| ---------------- | ----------- | ---------- | -------- |
| first 50 by id   | 180 883 365 | 2.34 GiB   | 413 ms   |
| `id % 7919 = 13` | 116 778 764 | 27.29 GiB  | 2 708 ms |

Against a 100 GB/hour read quota that is roughly **4–40 page views per hour**
before the quota is gone. The sort key does not save it: the page's ids are
scattered across the whole key space, so the seek touches granules everywhere.

**Conclusion: read-time derivation is viable only where the fact table is small
(`nft_ownership`, `nft_ownership_pending`) and self-contained
(`soroban_contracts`). For `accounts` and `lp_positions` it is out.**

## Approach — per column, by measured cost

### Small / self-contained → read-time derivation (the 0528 pattern)

| Column                                                 | Derivation                                                                                                                         |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `nfts_pending.minted_at_ledger`                        | `min(ledger_sequence)` over `nft_ownership_pending WHERE event_type = 0`                                                           |
| `soroban_contracts.deployed_at_ledger` + `deployer_id` | `min(wasm_uploaded_at_ledger)` + `argMin(deployer_id, …)` over rows with a non-NULL deployer — self-contained in a 185 k-row table |

### Large fact tables — the six options, and why four of them lose

`accounts.first_seen_ledger` and `lp_positions.first_deposit_ledger` are the hard
cases. All six candidates below were considered; **A and B were rejected on
measurement, not on taste.** The measurements are in the Cost section above and
repeated inline here so nobody re-proposes them from first principles.

|       | Option                                                            | Read cost                             | Write cost                               | Survives parallel backfill?               | Verdict                                                                       |
| ----- | ----------------------------------------------------------------- | ------------------------------------- | ---------------------------------------- | ----------------------------------------- | ----------------------------------------------------------------------------- |
| **A** | Derive at read time from the fact table (the 0528 pattern)        | **2.34–27.3 GiB per 50-account page** | none                                     | yes                                       | ✗ measured — 4–40 page views per hour against the quota                       |
| **B** | Writer reads the current minimum and preserves it                 | none extra                            | **98 MiB per 100 accounts**, every batch | **no**                                    | ✗ measured — and it still breaks under the 0228 scenario                      |
| **C** | Side `AggregatingMergeTree` + materialised view on the fact table | point lookup, 16 M × 2 columns        | incremental, **no read**                 | **no** — MVs do not fire on `ATTACH PART` | ✗ demoted; see the ATTACH discriminator below                                 |
| **D** | ClickHouse projection carrying the aggregate                      | same as C                             | same as C, maintained by CH              | yes — travels inside the part             | ✓ **recommended** — no extra table, but grows the parts                       |
| **E** | Change the `accounts` engine to `AggregatingMergeTree`            | none                                  | none                                     | yes                                       | most fundamental; largest blast radius (every reader and writer of the table) |
| **F** | `repair-tier1` on a schedule                                      | none                                  | periodic full pass                       | no                                        | ✗ plaster — the defect keeps producing between runs                           |

**Why C, D and E and not A or B:** they are the only shapes where **the merge
itself carries the MIN semantics**. Nothing is read at write time and nothing is
aggregated at read time, so there is no window in which a later event can
overwrite the historic minimum. That missing capability — RMT replacing a row
wholesale instead of folding it column-wise — is the root of the defect in all
six columns, so fixing it at the engine level fixes the class, not an instance.

Sketch for C:

```
account_first_seen (account_id, first_seen SimpleAggregateFunction(min, Int64))
  ENGINE = AggregatingMergeTree ORDER BY account_id
  ← MV on transaction_participants inserts
```

Measurement notes behind the rejections, so they are reproducible:

- **A** was measured twice on independent prod slices (2.34 GiB / 413 ms and
  27.29 GiB / 2 708 ms for 50 accounts). The sort key does not rescue it — a
  page's ids are scattered across the whole key space.
- **B** was measured with 100 **literal** StrKeys, i.e. the shape the indexer
  would actually issue: 1.74 M rows / 98 MiB / 24 ms. The sparse index pulls a
  whole granule per key, so the cost does not fall with batch size the way a
  point lookup would. An earlier `WHERE id IN (…)` variant read 17.8 M rows
  because `accounts` sorts on `account_id`, not on the `id` surrogate — worth
  knowing before anyone re-measures.

#### Backfill cost — measured, and it is not the obstacle

One partition each, on prod, extrapolated to the full table:

| Backfill                                  | Rows    | Read     | Compute | Peak memory |
| ----------------------------------------- | ------- | -------- | ------- | ----------- |
| `accounts` ← `transaction_participants`   | 10.74 B | ~160 GiB | ~17 s   | **94 MiB**  |
| `lp_positions` ← `operations_appearances` | 6.85 B  | ~46 GiB  | ~10 s   | **27 MiB**  |

Measured slices: partition 101 of `transaction_participants` = 498 551 887 rows /
7.43 GiB / 796 ms / 94.28 MiB; the same partition of `operations_appearances`
filtered to `type = 22` with `arrayJoin(pool_ids)` = 246 249 347 rows / 1.66 GiB /
368 ms / 26.53 MiB.

Memory is a non-issue — the aggregation streams in sort-key order. Compute is
seconds. **The only constraint is the read quota** (2 B rows / 100 GB per
server-hour), which a single full pass exceeds. Chunk it by partition: 90 chunks
of ~7.4 GiB for `accounts`. That is a routine maintenance job, not a blocker.

Note the contrast that makes the whole case: the same aggregate costs 2.34–27.3
GiB **per page view** as option A, and ~160 GiB **once, ever** as a backfill.

#### The `ATTACH PART` discriminator — this demotes C

The parallel backfill does not `INSERT`. Per ADR 0045 / task 0228 it uses
**FREEZE + rsync + ATTACH PART** (`docs/backfills.md` §B2). ClickHouse
materialised views are INSERT triggers, so:

- **C** — the MV would **not fire at all** for backfilled data. The side table
  would be silently incomplete, reproducing exactly the defect it exists to fix.
  C is only safe if the backfill procedure gains an explicit re-materialisation
  step, which puts a manual, forgettable step back on the critical path — the
  same failure shape as today's `repair-tier1`.
- **D** — a projection lives inside the part. A part frozen on a worker whose
  table carries the same projection arrives with it, so `ATTACH PART` brings the
  aggregate along. Requires worker and target schemas to stay in step, which the
  procedure already requires.
- **E** — attached parts carry raw rows and the `AggregatingMergeTree` merge
  folds them with `min()`. Correct by construction, no procedural step at all.

**Revised recommendation: D or E, not C.** C looked cheapest until the write path
was checked; it fails on the very scenario (parallel backfill) that opened 0228.

#### Verified by experiment, not by documentation

Run on a local ClickHouse 26.3, reproducing the FREEZE → copy → `ATTACH PART`
sequence the backfill actually uses.

**1. A materialised view does NOT fire on `ATTACH PART`.**
`src` (MergeTree) with an MV into `mvtarget`. Control first: `INSERT` of 3 rows
→ `mvtarget` = 3, so the MV works. Then 5 rows written to a separate `worker`
table, `FREEZE`, part copied into `src/detached/`, `ATTACH PART`:

|            | before | after ATTACH |
| ---------- | ------ | ------------ |
| `src`      | 3      | **8**        |
| `mvtarget` | 3      | **3**        |

The rows arrived; the view never ran. **Option C is dead** — its side table would
be silently short by exactly the backfilled range.

**2. A projection travels inside the frozen part and arrives usable.**
`p1` with `PROJECTION pmin (SELECT id, min(seq) GROUP BY id)`; the frozen part
contains a `pmin.proj` subdirectory. Copied into `p2` (same projection declared)
and attached: `system.projection_parts` lists `pmin` for `p2`, the aggregate
returns the correct minima, and the query still succeeds under
`SETTINGS force_optimize_projection = 1` — so it is genuinely used, not merely
present. **Option D holds.**

**3. `AggregatingMergeTree` folds correctly, and is right even before merging.**
Three separate parts carrying `first_seen` values 900, 100, 500 for the same key.
Reading **without** `FINAL` already returns 100; after `OPTIMIZE FINAL` the table
holds one row with 100.

That last point matters more than it looks. Prod RMT tables are chronically
un-merged (task 0420), so anything that depends on merges having run is fragile
here. A `min` aggregate does not: unmerged parts produce extra rows that the
query's own `min()` folds correctly. **Option E is robust against the exact
un-merged-parts condition that already bites this cluster.**

Remaining open:

- `SimpleAggregateFunction(min, …)` was the shape tested and it behaved
  correctly; `minState`/`minMerge` was not compared.
- **D vs E is the only decision left.** Both survive `ATTACH PART`, both were
  verified working. D is contained — one projection, no engine change, no
  migration of `accounts`. E is more fundamental: the table itself stops being
  able to express a wrong minimum, and it needs no `FINAL`, which is worth real
  money on a cluster whose RMT tables do not merge. E costs a full rewrite of
  `accounts` and touches every reader and writer of it.
- Decide on blast radius, not on correctness — correctness is settled for both.

### Two traps 0528 hit, certain to recur

- `min()` over a non-Nullable column returns a **non-Nullable** type, and without
  `join_use_nulls = 1` (unavailable — `api_reader` is readonly) a LEFT JOIN miss
  fills the type DEFAULT rather than NULL. Wrap in `nullIf(_, 0)`, or the
  endpoint 500s on decode and a missing value renders as "ledger 0". This is not
  hypothetical: `nfts` held a literal stored `0` that the old code displayed as
  "Minted at ledger 0".
- Where the value is a sort key or cursor key, the ORDER BY, the keyset predicate
  and the cursor payload must move together, or pagination stops being total.

## Absorbed from 0529 — drop every dead column in ONE window

Once nothing reads the six Tier-1 columns, they and the four long-vestigial
metadata columns can go. Paying the `ALTER` window once is the whole reason 0529
was folded in here.

- Strip the fields from the row structs (`db-clickhouse/src/persist/rows.rs`) and
  from the staging merge (`persist/stage.rs`) — including the in-batch `min()`
  folds, which become dead with them.
- Delete the corresponding halves of `repair_tier1`; retire the subcommand once
  nothing it repairs is read.
- Remove the columns from `db-clickhouse/schema/init.sql`.
- **One** production `ALTER` dropping: `nfts.minted_at_ledger`,
  `nfts_pending.minted_at_ledger`, `accounts.first_seen_ledger`,
  `lp_positions.first_deposit_ledger`,
  `soroban_contracts.{deployer_id, deployed_at_ledger}` _(only those actually
  superseded — decide per column)_, plus the already-vestigial
  `nfts.{name, media_url, collection_name}` and the `nfts_pending` twins.

### Deploy ordering — the real hazard

0310 cost ~9 minutes of ingest stall here. The `clickhouse` 0.15 driver validates
the row struct against `DESCRIBE TABLE` **in both directions**:

- slimmed struct + column still present (and no `DEFAULT`) → client-side
  `SchemaMismatch`, inserts fail
- struct still carrying the field + column already dropped → also a mismatch

Neither "deploy first" nor "ALTER first" is safe alone, and warm Lambda
containers cache the `DESCRIBE` until a config-touch recycles them.

**Untested idea worth trying before booking the window:** give each column an
explicit `DEFAULT` first
(`ALTER TABLE … MODIFY COLUMN … DEFAULT NULL`). 0310's failure was specifically
_"table columns without DEFAULT not covered by the struct"_, so a defaulted
column may let the slimmed struct pass while the column still exists — collapsing
the window to zero. **Verify against the driver before relying on it.**

## Ordered steps

1. Deploy 0528 so the NFT fix reaches users. **Not a blocker for steps 2–4** —
   it only gates step 5, because the `ALTER` must follow the deploy of code that
   stopped reading.
2. Read-time derivation for `nfts_pending` and `soroban_contracts` (cheap, the
   0528 pattern).
3. Maintained MIN aggregate for `accounts`, then `lp_positions`: build the MV,
   backfill it, measure, then switch the read.
4. Strip the writer: fields out of the row structs, the staging merge, the schema
   and `repair_tier1`.
5. One production `ALTER` window dropping every dead column, ordered per the
   hazard above; verify ingest health and no ledger gap.
6. Retire the `repair-tier1` subcommands that no longer repair anything.
7. Correct the documentation premise: `repair_tier1.rs` header, task 0228, and
   0322 item (G) all still say "parallel backfill".

## Acceptance Criteria

- [ ] Every one of the six columns is either served from a correct source, or has
      a written decision recording why not and what replaces it
- [ ] The C / D / E choice for the large fact tables is recorded with the
      backfill measurement that decided it — A and B stay rejected unless a new
      measurement overturns them
- [ ] Option B from "How to present a value the chain never recorded" is shipped
      or explicitly declined: a mint ledger the chain never carried reads as
      "not recorded on chain", not as an empty field
- [ ] `accounts` list and detail measured before/after — no page-latency or
      read-quota regression on the hot path
- [ ] The maintained aggregate proven correct against a fact-table sample, and
      proven to fire for the bulk-insert path as well as live ingest
- [ ] Keyset pagination proven total wherever a corrected value is a cursor key
- [ ] Prod measurement before and after, per column, including a re-measure of
      the `accounts` 400-row sample
- [ ] One `ALTER` window, ingest verified healthy afterwards — no
      `SchemaMismatch`, no ledger gap
- [ ] `repair-tier1` retired or reduced to what it still owns
- [ ] `repair_tier1.rs`, task 0228 and 0322 item (G) corrected
- [ ] **Docs updated** — endpoint-query files for every touched endpoint, and the
      schema docs for the dropped columns
- [ ] **API types regenerated** — expected `N/A` (sources change, wire shape does
      not); confirm with an actual empty diff

## Known limits of the derivation itself

A MIN over a fact table can only be as complete as the facts. Measured on prod:
one `nfts` token and 66 `nfts_pending` tokens exist with **no Mint row at all**.
Investigated for the `nfts` case (contract `CCIP47L5…NWNH`, token 2): 219 events,
17 `mint` events covering tokens 0 and 3–18, none for tokens 1–2, across **both**
event encodings the contract used (map-shaped and scalar-shaped) — and the
parser dropped nothing, 17 events producing 17 rows. The contract was deployed at
ledger 60 908 576, far above our floor of 50 457 424, so coverage is not the
cause: it simply never emitted a creation event for those tokens.

### How to present a value the chain never recorded

Four options were weighed:

|       | Option                                                                   | Verdict                                                       |
| ----- | ------------------------------------------------------------------------ | ------------------------------------------------------------- |
| **A** | Serve NULL, render blank (behaviour since 0528)                          | ✓ correct, already shipped                                    |
| **B** | A, plus the UI says **"not recorded on chain"** instead of a blank field | ✓ **recommended addition**                                    |
| **C** | Fall back to the earliest ownership row                                  | ✗ reports a transfer ledger as a mint ledger                  |
| **D** | Ask the chain over RPC                                                   | ✗ not possible — the contract's state is archived (see below) |

**B matters more than the `nfts` count suggests.** One token in `nfts`, but
**66 of 277 (24%) in `nfts_pending`** — and pending rows get promoted. A blank
field reads as "no value"; "not recorded on chain" states the actual fact, which
is that the chain never carried it. This is the same rule the project already
applies elsewhere: an empty section must say why it is empty, never impersonate a
real empty value.

Scope note: B is a small frontend + DTO change and can ship independently of the
storage work in this task. Do not let it block steps 2–5.

Serving NULL there is correct and must stay correct — an invented `0` is worse
than an honest blank. Do not "fix" these by falling back to the earliest
ownership row; that would report a transfer ledger as a mint ledger.

Note for anyone verifying such a case against the chain: `owner_of` on that
contract now fails with contract error #200 for **every** token, including ones
with recent activity, which is consistent with Soroban state archival rather than
with the tokens not existing. Contract error codes are not a reliable oracle for
"this token was burned".

## Notes

- Do not treat this as urgent-and-total. `accounts` carries the real user
  exposure and the real cost risk; the small tables are nearly free. Sequence by
  exposure, and measure before committing to an approach on the two big fact
  tables.
- The production `ALTER` and the deploy are the operator's to run, not the
  agent's.
