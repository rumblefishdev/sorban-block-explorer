# Repair: `liquidity_pools.legs` (task 0374)

Two one-off repairs of the SAME column, by the same mechanism, on disjoint
scopes. Run both in one window:

|       | scope           | what is wrong                                                  |
| ----- | --------------- | -------------------------------------------------------------- |
| **A** | `pool_kind = 1` | soroban legs keyed on the SAC surrogate, resolving to no asset |
| **B** | `pool_kind = 0` | classic legs never filled, because nothing rewrote the row     |

Repair B was briefly a Rust pass in `backfill-runner` (whole-table rebuild +
`EXCHANGE`, indexer stopped). That was wrong twice over: it duplicated this
runbook's mechanism on the same column, and its premise was false. The premise
was "our leg surrogate is a hash ClickHouse cannot compute, so it must be Rust"
— true, and beside the point. For a classic pool the leg surrogate IS the
`assets.id` of that asset (identical formula, identical inputs), so the value is
not computed, it is **looked up** — which SQL does perfectly well. Verified on
production: the expression in B reproduces the stored legs for **44,106 of
44,108** pools that already have them; the two misses are pools whose asset has
no `assets` row at all, and B leaves those alone.

## What is wrong and why this exists

The three soroban registry-row builders keyed every leg on the token's own
contract surrogate. That is right for a Soroban-native token — its contract
surrogate IS its `assets.id` — and wrong for a SAC: ADR 0051 retired
`asset_type = 2`, so a SAC has no `assets` row of its own and a leg keyed on
its surrogate points at nothing.

Measured on production 2026-09-08: **1,084 of 1,175 soroban leg occurrences
resolved to no asset at all**, while 38,932 of 38,932 classic legs resolved
cleanly. 218 pools hold a native XLM leg and not one carried XLM's id.

The writer is fixed (`contract_token_asset_id` in `persist/stage.rs`, commit
`bf1a46a8`). This runbook repairs the rows written before that fix.

## Preconditions — the order is the whole safety argument

A ClickHouse mutation rewrites the parts that exist when it starts. Rows
inserted afterwards are untouched. So **every writer still producing old-rule
rows must be finished before step 3**, or it re-introduces them.

1. **The registry backfill has finished.** It emits `liquidity_pools` rows for
   BOTH pool kinds, so a run in flight is such a writer.
2. **The corrected writer is deployed.** From that moment the live indexer
   writes correct legs, which is what makes step 3 safe without a pause.
3. Only then: run the repair.

**No indexer pause is needed** — that is what the ordering buys. A pause would
be required only if the mutation ran before the writer was deployed.

**Standing hazard, permanent:** running any backfill with a binary older than
`bf1a46a8` after this repair re-breaks the column. Precondition 2 applies to
every later run, not just this one.

## A — the soroban rewrite expression

Used identically by the dry run and the mutation, so what you verify is what
you run:

```sql
arrayMap(
  (x, y) -> if(y = 0, x, y),
  legs,
  arrayMap(x ->
    (SELECT CAST(groupArray((s.sacid, a.aid)), 'Map(Int64, Int64)')
     FROM (SELECT sac_contract_id AS sacid,
                  any(asset_type) t, any(asset_code) c, any(issuer_id) i
           FROM asset_sac WHERE sac_contract_id != 0
           GROUP BY sac_contract_id) AS s
     INNER JOIN (SELECT asset_type t, asset_code c, issuer_id i, any(id) aid
                 FROM assets WHERE asset_type IN (0, 1)
                 GROUP BY asset_type, asset_code, issuer_id) AS a
       ON a.t = s.t AND a.c = s.c AND a.i = s.i)[x],
    legs))
```

Notes on the shape, each one deliberate:

- The map is built from `asset_sac` LIVE, not from a pasted snapshot, so it
  cannot go stale between generating and running.
- A missing key yields `0`, and `0` is never a valid leg — hence the
  `if(y = 0, x, y)` fallthrough, which leaves Soroban-native legs alone.
- The subquery appears **once**. Two `groupArray` subqueries feeding
  `transform(x, keys, values, x)` would also work, but their row order is not
  guaranteed to match between two independent executions — that would silently
  transpose the mapping. The `Map` form has no such hazard.
- No `OPTIMIZE ... FINAL` afterwards: a mutation rewrites parts in place rather
  than inserting a new version, so there is no version tie to merge away. It
  also updates the RMT's duplicate physical rows, which is what we want.

## B — the classic fill expression

`legs` is filled at WRITE time and `liquidity_pools` is a ReplacingMergeTree
keyed on `pool_id`, so a pool gets its legs when the indexer next touches it —
and a pool that stopped trading never does. Measured 2026-09-09: 8,825 classic
pools still empty, none touched in the previous week, so no ledger-range
re-index reaches them however long it runs. This is what closes that gap, and
what makes the pair-column drop gate reachable.

```sql
arrayMap(x -> if(x = 0, -1, x), [
  (SELECT map FROM legmap)[concat(toString(if(asset_a_type = 0, 0, 1)), ':',
                                  if(asset_a_type = 0, '', toString(asset_a_code)), ':',
                                  toString(if(asset_a_type = 0, 0, asset_a_issuer_id)))],
  (SELECT map FROM legmap)[concat(toString(if(asset_b_type = 0, 0, 1)), ':',
                                  if(asset_b_type = 0, '', toString(asset_b_code)), ':',
                                  toString(if(asset_b_type = 0, 0, asset_b_issuer_id)))]
])
```

where `legmap` is

```sql
SELECT CAST(groupArray((k, id)), 'Map(String, Int64)') AS map FROM (
  SELECT concat(toString(asset_type), ':', toString(asset_code), ':',
                toString(issuer_id)) AS k, id
  FROM assets WHERE asset_type IN (0, 1) LIMIT 1 BY id
)
```

Notes, each one deliberate:

- The key is the identity tuple, not the surrogate — that is the whole point:
  the hash was already computed once, when the `assets` row was written.
- `asset_type` is the XDR domain in the pair columns (`1` and `2` are both
  classic credit) and the FAMILY domain in `assets` (`1` = classic credit), so
  the key folds `1|2 → 1`. Getting this backwards is task 0489's defect, which
  blanked one leg of every trade on 59% of pools.
- **A map miss must never be written.** `0` is not a valid leg, and a stored
  `0` is worse than an empty array: it looks like an answer. The guard below
  makes such a row keep its empty `legs` and stay visible to the gate.

## 1. Dry run (read-only, safe to repeat)

Substitute `<REWRITE>` with the expression above.

```sql
-- How much moves.
SELECT count() AS pools,
       countIf(legs != <REWRITE>) AS pools_changed,
       sum(length(legs)) AS legs_total,
       sum(arrayCount(i -> legs[i] != (<REWRITE>)[i],
                      range(1, length(legs) + 1))) AS legs_changed
FROM (SELECT pool_id,
             argMax(pool_kind, last_updated_ledger) AS k,
             argMax(legs, last_updated_ledger) AS legs
      FROM liquidity_pools GROUP BY pool_id)
WHERE k = 1
```

Expected at the 2026-09-08 measurement: `582 | 551 | 1175 | 1083`.
Those numbers grow as new pools register — what matters is that
`legs_changed` equals the mis-keyed count from the hard invariant below.

## 2. Dry run of the invariants (read-only)

Both checks, against the state the mutation WOULD produce:

```sql
WITH t AS (SELECT <REWRITE> AS new_legs
           FROM (SELECT pool_id,
                        argMax(pool_kind, last_updated_ledger) AS k,
                        argMax(legs, last_updated_ledger) AS legs
                 FROM liquidity_pools GROUP BY pool_id)
           WHERE k = 1),
     fl AS (SELECT arrayJoin(new_legs) AS leg FROM t),
     sac AS (SELECT sac_contract_id AS sid FROM asset_sac
             WHERE sac_contract_id != 0 GROUP BY sac_contract_id),
     an AS (SELECT id, 1 AS found FROM assets GROUP BY id)
SELECT count()                    AS legs_after,
       countIf(sac.sid != 0)      AS hard_still_keyed_on_a_sac,
       countIf(an.found != 1)     AS soft_no_assets_row
FROM fl LEFT JOIN sac ON sac.sid = fl.leg
        LEFT JOIN an  ON an.id  = fl.leg
```

Measured 2026-09-08: `1175 | 0 | 1`.

- **`hard_still_keyed_on_a_sac` must be 0.** This is the defect itself; it has
  no legitimate non-zero value, before or after.
- **`soft_no_assets_row` is a counter, not a gate.** A leg can legitimately
  resolve to nothing when we have never observed its token at all. Today that
  is exactly one: pool
  `8FE06922A146D7BEBAB9CDD93D0E34224AFE09CFB42465378423F26DA8DE3370`,
  registered at ledger 50,875,676 by one of the five dead early config-factory
  deployments. It holds one `pool_state_changes` row (its creation reserves),
  emitted 2 events ever, and its leg token has emitted **zero** — above the
  ingest floor (50,457,424), simply inert. If this number climbs, a token
  family is escaping the dimension writers; investigate rather than relax the
  check.

Do not merge the two into one "every leg resolves in `assets`" check. It cannot
pass while that dead pool exists, and the pressure would then be to weaken it
until it catches nothing.

## 3. The mutations (WRITE — operator only)

```sql
-- A: soroban
ALTER TABLE liquidity_pools UPDATE legs = <REWRITE A> WHERE pool_kind = 1;

-- B: classic, only where a fill is needed AND both legs resolve. The
-- `NOT has(..., -1)` guard is why the expression maps a miss to -1: a row
-- whose asset has no `assets` row is skipped entirely rather than written
-- with a placeholder.
ALTER TABLE liquidity_pools UPDATE legs = <REWRITE B>
WHERE pool_kind = 0 AND length(legs) = 0 AND NOT has(<REWRITE B>, -1);
```

Single-server `ReplacingMergeTree`, no replication, so no `ON CLUSTER`.

## 4. Watch it

```sql
SELECT mutation_id, command, parts_to_do, is_done, latest_fail_reason
FROM system.mutations
WHERE table = 'liquidity_pools'
ORDER BY create_time DESC LIMIT 3
```

`is_done = 1` and an empty `latest_fail_reason` mean it finished. Mutations are
asynchronous: the statement returns before the work does.

## 5. Verify

Re-run the invariant query from step 2 **without** the rewrite — read `legs`
directly. `hard_still_keyed_on_a_sac` must be `0`, and `soft_no_assets_row`
must be the same small number the dry run predicted.

Then confirm the read-side consequence, which is the point of the whole
exercise — a native XLM leg now matches classic pools:

```sql
SELECT countIf(has(legs, -6959166271784855184)) AS soroban_pools_with_native_xlm
FROM (SELECT argMax(pool_kind, last_updated_ledger) AS k,
             argMax(legs, last_updated_ledger) AS legs
      FROM liquidity_pools GROUP BY pool_id)
WHERE k = 1
```

Before: `0` — verified read-only on production. After: `218` when the rewrite
is applied to today's rows. That number grows as pools register; `0` before is
the part that must hold.

For **B**, the gate the pair-column drop depends on:

```sql
SELECT countIf(length(legs) = 0)  AS unmigrated,   -- target 0
       countIf(has(legs, 0))      AS placeholders  -- MUST be 0, always
FROM liquidity_pools FINAL
```

`unmigrated` should fall to the handful of pools whose asset has no `assets`
row (2 on production, 2026-09-09) — B skips those by design rather than writing
a placeholder. `placeholders` must be `0`: a stored `0` is not a leg, and it
would read as an answer instead of as missing data.

Only once `unmigrated` is at that floor may the legacy pair columns be dropped
(see `docs/deployment.md`) — B reads the very columns that migration removes,
so it can never be re-run afterwards.

## Rollback

There is none in the "undo the mutation" sense — a mutation is destructive of
the previous values. It does not need one: the old values are recomputable from
the registration events, which sit complete in `soroban_events` forever, and
the corrected writer reproduces the new values from the same source. If the
repair were ever judged wrong, the answer is a registration re-parse, not an
inverse mutation.
