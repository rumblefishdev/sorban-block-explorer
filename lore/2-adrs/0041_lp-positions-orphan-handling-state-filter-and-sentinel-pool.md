---
id: '0041'
title: 'lp_positions orphan handling: liquidity_pool `state` filter + sentinel placeholder pool'
status: accepted
deciders: [stkrolikiewicz]
related_tasks: ['0189']
related_adrs: ['0027', '0031', '0032', '0037']
tags: [schema, indexer, partial-backfill, lp-positions, sentinel]
links:
  - crates/xdr-parser/src/state.rs
  - crates/indexer/src/handler/persist/write.rs
  - crates/db/migrations/0006_liquidity_pools.sql
  - crates/audit-harness/sql/15_liquidity_pools.sql
history:
  - date: 2026-05-05
    status: accepted
    who: stkrolikiewicz
    note: 'ADR created — captures lore-0189 fix conventions for orphan lp_positions in partial backfills.'
---

# ADR 0041: lp_positions orphan handling: liquidity_pool `state` filter + sentinel placeholder pool

**Related:**

- [Task 0189: BUG lp_positions FK violation when parent pool not extracted in same ledger](../1-tasks/active/0189_BUG_lp-positions-fk-violation-pool-not-extracted.md)
- [ADR 0027: Initial schema](./0027_*.md)
- [ADR 0031: asset\_\*\_type SMALLINT](./0031_enum-columns-smallint-with-rust-enum.md)
- [ADR 0032: docs/architecture/\*\* evergreen](./0032_docs-architecture-evergreen-maintenance.md)
- [ADR 0037: Current schema snapshot](./0037_current-schema-snapshot.md)

---

## Context

The bridge backfill that followed lore-0185 crashed at ledger `62148003` with a
PostgreSQL `23503` foreign-key violation:

```
Key (pool_id)=(\xd631…2a561) is not present in table "liquidity_pools".
```

Two parallel extractors over the same `LedgerEntryChanges` slice produced an
asymmetric outcome:

| Function (`crates/xdr-parser/src/state.rs`) | Filter                                                 | Pre-fix `change_type` skips |
| ------------------------------------------- | ------------------------------------------------------ | --------------------------- |
| `extract_liquidity_pools`                   | `entry_type="liquidity_pool"`                          | **`state`, `removed`**      |
| `extract_lp_positions`                      | `entry_type="trustline"` AND `asset.type="pool_share"` | **`state`**                 |

When an account opened, modified, or closed a pool_share trustline in a ledger
that did not also mutate the pool's reserves, the trustline carried a normal
`created/updated/removed` change while the pool itself surfaced only as a
`state` snapshot in op_meta (or not at all if it was created in a pre-window
ledger and untouched in the current one). The position emitted, the pool did
not, and `lp_positions.pool_id`'s FK to `liquidity_pools.pool_id` tripped.

Empirically (see lore-0189 task investigation), the d63184 reproducer at
ledger 62148003 has the pool present as `change_type="state"` with full real
data (asset_a=Lira/credit_alphanum4, asset_b=liragold/credit_alphanum12,
fee=30bps); the trustline is `change_type="removed"`. So the most common
case is "pool visible as state, position is real" — fixable by loosening the
extractor filter.

For the residual case (pool not in the current ledger at all — created in a
pre-window ledger and not touched again until later), the extractor cannot
emit pool dimension data because the trustline alone carries only the
pool_id, not the asset pair or fee. The schema requires
`asset_a_type, asset_b_type, fee_bps, created_at_ledger NOT NULL`
(`crates/db/migrations/0006_liquidity_pools.sql:16–28`); we cannot insert a
position without a parent row.

Existing precedent for the same problem class on a sibling table —
`operations_appearances.pool_id` — uses **nullify** (`write.rs:777–796`,
"Nullify pool_id when the referenced pool is not present; the op row stays,
only the FK link turns NULL"). That pattern does not apply here:
`lp_positions.pool_id` is part of the composite `PRIMARY KEY (pool_id,
account_id)` and is `NOT NULL`. Removing it from the PK changes row identity
(an account holds positions across many pools).

---

## Decision

Two changes, both behavioral; no schema migration.

### 1. Extractor — include `state` in the `extract_liquidity_pools` filter

`extract_liquidity_pools` (`crates/xdr-parser/src/state.rs`) accepts pool
entry changes whose `change_type` is in
`{created, updated, restored, state}`. The previous set excluded `state`.

Rationale: a `state` snapshot in op_meta is the read-only "current entry"
view Stellar Core writes when an operation references an entry without
modifying it. It carries full LedgerEntry data — same shape as
`created/updated/restored` — so we can extract the full pool dimension from
it. Snapshots emitted alongside are absorbed by the existing
`liquidity_pool_snapshots` UPSERT (`uq_lp_snapshots_pool_ledger DO NOTHING`,
`write.rs:1702`); first-write-wins is safe because state views in op_meta
carry identical data per `(pool_id, ledger_sequence)`.

`extract_account_states` retains its `state` skip — that filter is about
balance change semantics ("observation-only, no balance change"), not
dimension extraction. The two extractors have different concerns; loosening
state for pool dimensions does not cascade.

### 2. Persist — sentinel placeholder pool for orphan positions

`crates/indexer/src/handler/persist/write.rs::upsert_pools_and_snapshots`
gains a pre-13a step:

1. `detect_orphan_pool_ids` collects every `pool_id` referenced by
   `staged.lp_position_rows` that is NOT in `staged.pool_rows` and NOT in the
   `liquidity_pools` table (single batched `WHERE pool_id = ANY($1)`).
2. `insert_sentinel_pools` writes a placeholder row for each orphan with
   marker convention `created_at_ledger = 0` and minimum-data
   sentinel fields (`asset_a_type=0, asset_a_code=NULL, asset_a_issuer_id=NULL,
asset_b_type=0, asset_b_code=NULL, asset_b_issuer_id=NULL, fee_bps=0`).
   `ON CONFLICT (pool_id) DO NOTHING` — idempotent; no overwrite of real or
   earlier-sentinel rows.
3. The 13a `liquidity_pools` UPSERT carries sentinel-aware `ON CONFLICT DO
UPDATE`: when existing has `created_at_ledger=0` and incoming has
   `created_at_ledger > 0`, every dimension field is replaced with EXCLUDED.
   Otherwise existing real values are preserved (no downgrade).

Sentinel marker selection — `created_at_ledger = 0`:

| Property                                                         | Evidence                          |
| ---------------------------------------------------------------- | --------------------------------- |
| Stellar pubnet ledger sequence ≥ 1 (genesis = 1, July 2015)      | Protocol-level, never violated.   |
| 0 existing `liquidity_pools` rows have `created_at_ledger=0`     | Verified at planning (lore-0189). |
| Single column → simple detection (`WHERE created_at_ledger = 0`) | Minimal ambiguity.                |
| Existing `BIGINT NOT NULL` accepts 0                             | No migration required.            |

### 3. Audit invariant — informational placeholder count

`crates/audit-harness/sql/15_liquidity_pools.sql` gains
`I6 — sentinel placeholder pool count`. Counts rows with
`created_at_ledger = 0`. Not a violation; a thermometer for partial-backfill
coverage. Should converge to 0 on from-genesis backfills.

The FK-consistency invariant (`17_lp_positions.sql:I1`) already exists and
is not duplicated.

---

## Rationale

The combined approach preserves all data the extractor can derive from the
ledger meta we have, falls back to a marked placeholder only when no
dimension data is available in the current persistence transaction, and
self-heals in place when real data later arrives via the normal
`extract_liquidity_pools` path.

Key properties:

- **No schema migration.** Existing CHECK constraints (`ck_lp_pool_id_len`,
  `ck_lp_asset_*_type_range`, no constraint on `created_at_ledger`) accept
  sentinel rows.
- **No new column.** Marker uses an existing field with a value no real row
  can carry.
- **Idempotent across runs.** Both `insert_sentinel_pools` (DO NOTHING) and
  the 13a UPSERT (sentinel-aware DO UPDATE) handle every existing/incoming
  combination correctly, including no-op replays.
- **Backward compatible.** Existing UPSERT semantics for
  `(real, real)` collapses to the prior `LEAST(...)` behavior plus a
  no-op `existing.asset_a_type` update.
- **Existing invariants tolerant.** `15_liquidity_pools.sql:I1–I5` and
  `17_lp_positions.sql:I1–I6` either pass on sentinel rows or skip them
  cleanly (verified in lore-0189 plan).

---

## Alternatives Considered

### Alternative 1: Skip orphan positions entirely

**Description:** When an `lp_positions` row references a missing pool, drop
it with a `warn!` log; do not emit a pool row.

**Pros:** Simpler — no sentinel logic, no UPSERT changes.

**Cons:** Position data permanently lost for accounts whose pool dimension
was never observable in the backfill window. A partial-backfill DB would
miss real participants of pre-window pools — opaque data loss with only a
warn-log trail.

**Why not:** Sentinel preserves the position with explicit, queryable marker
and self-heals. The cost (a small extra UPSERT branch) is dwarfed by the
information preserved.

### Alternative 2: New `is_placeholder` boolean column

**Description:** Schema migration adding `is_placeholder BOOLEAN NOT NULL
DEFAULT FALSE`; sentinel rows set it true; UPSERT upgrades on real data
arrival.

**Pros:** Explicit marker, no convention via existing column values.

**Cons:** Schema change; touches every consumer (5 endpoint queries +
analytics) that already filter or display pool fields; cross-codebase
coordination cost. User explicitly declined: "nie chce rozszerzać schemy o
dodatkową kolumnę is_placeholder."

**Why not:** Existing `created_at_ledger` is already unambiguous for the
sentinel range (only value Stellar protocol cannot produce); new column is
unnecessary.

### Alternative 3: Nullify `lp_positions.pool_id` like `operations_appearances`

**Description:** Drop NOT NULL on `lp_positions.pool_id`; null when parent
missing, mirroring the `write.rs:777–796` pattern.

**Pros:** Existing pattern in the codebase, low surface area.

**Cons:** `pool_id` is part of `lp_positions`'s composite PRIMARY KEY. NULL
in a PK is forbidden. Removing it from the PK changes identity (an account
holds positions across many pools). Not viable.

### Alternative 4: Network-side dimension fetch (Horizon / Stellar archive)

**Description:** When orphan detected, fetch pool dimension from external
service before insert.

**Pros:** Real data preserved.

**Cons:** Hot-path network call in the persist transaction; retry chains;
external dependency; throughput collapse during backfill. Not acceptable.

---

## Consequences

### Positive

- Orphan FK violations stop crashing the indexer.
- Position participation history is preserved across partial backfills.
- Sentinel placeholders are a queryable, self-healing transient state with
  an explicit marker (`created_at_ledger = 0`).
- Existing audit invariants stay green; new I6 surfaces partial-backfill
  coverage as a metric.

### Negative

- Pool dimension API endpoints (`docs/architecture/database-schema/endpoint-queries/18_get_liquidity_pools_list.sql`,
  `19_get_liquidity_pools_by_id.sql`, `20_get_liquidity_pools_transactions.sql`,
  `21_get_liquidity_pools_chart.sql`, `23_get_liquidity_pools_participants.sql`)
  currently surface sentinel rows as "native+native, fee=0, ledger 0" — not
  useful to consumers. Out of scope for lore-0189; spawn a follow-up to
  filter (`WHERE created_at_ledger > 0`) or annotate via API response flag.
- Sentinel rows imply a semantically invalid Stellar pair (native+native at
  fee=0) that is conceptually inconsistent with protocol semantics. The
  marker convention (`created_at_ledger=0`) unambiguously distinguishes
  them, but consumers must be aware.

### Neutral

- New `I6` invariant is informational only. Production telemetry can wire it
  as a count metric; alarms are optional.

---

## Compliance with related ADRs

- **ADR 0027 (initial schema):** Schema unchanged. New convention
  (`created_at_ledger = 0` as sentinel marker) adds semantics on top of an
  existing column.
- **ADR 0031 (asset\_\*\_type SMALLINT):** Sentinel `asset_a_type = 0` /
  `asset_b_type = 0` are valid `AssetType::Native` values within the
  declared 0–15 range. Convention layered on top.
- **ADR 0037 (current schema snapshot):** Snapshot remains current. This ADR
  is an addendum documenting a column-level convention.
- **ADR 0032 (`docs/architecture/**` evergreen):** Architecture docs
updated in the same PR (`docs/architecture/database-schema/database-schema-overview.md`,
`docs/architecture/indexing-pipeline/indexing-pipeline-overview.md`).

---

## Notes

- Step 1 of the lore-0189 implementation (`crates/xdr-parser/examples/decode_pool_ledger.rs`)
  is a one-off diagnostic; kept in `examples/` to allow running the same
  check on any future ledger / pool_id pair without rebuilding the
  investigation tooling.
- Production indexer (Galexie / Lambda live) does not hit this path because
  it has full-from-genesis history. Sentinel logic is exercised only by
  partial / mid-stream backfill scenarios and re-runs against scratch DBs.
