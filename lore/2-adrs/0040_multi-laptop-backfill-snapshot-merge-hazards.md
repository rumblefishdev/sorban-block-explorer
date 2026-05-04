---
id: '0040'
title: 'Multi-laptop backfill snapshot merge — schema hazards and playbook'
status: proposed
deciders: [fmazur]
related_tasks: []
related_adrs: ['0010', '0026', '0027', '0030', '0035', '0036', '0037', '0038']
tags: [backfill, schema, merge, postgres, surrogate-keys, partitioning]
links: []
history:
  - date: 2026-05-04
    status: proposed
    who: fmazur
    note: 'ADR created — captures schema-level hazards before writing the multi-laptop snapshot merge script'
---

# ADR 0040: Multi-laptop backfill snapshot merge — schema hazards and playbook

**Related:**

- [ADR 0010 — Local backfill over Fargate](./0010_local-backfill-over-fargate.md)
- [ADR 0026 — Accounts surrogate BIGINT id](./0026_accounts-surrogate-bigint-id.md)
- [ADR 0030 — Contracts surrogate BIGINT id](./0030_contracts-surrogate-bigint-id.md)
- [ADR 0037 — Current schema snapshot](./0037_current-schema-snapshot.md)

---

## Context

We plan to parallelise a fresh historical backfill across N laptops. Each
laptop runs `backfill-runner` against its own local Dockerised Postgres on
disjoint ledger ranges (laptop1: 0–2M, laptop2: 2M–4M, …). When all laptops
finish, we will snapshot each DB and merge them into a single canonical
database.

The indexer was designed for a single-writer, monotonic-ledger pipeline; its
upsert semantics are replay-idempotent for the same DB but not for merging
independently-grown DBs. Before we write the merge script we need a complete
picture of:

- which tables are append-only vs upsert and on what conflict target,
- which surrogate keys are DB-local (BIGSERIAL) and which are natural,
- which tables hold "current state" that depends on monotonic ledger ordering,
- where cross-ledger references will dangle when a laptop never sees the
  ledger that created the referent (e.g. an asset minted at L=500k used at
  L=3M),
- which schema invariants will reject naive `INSERT … SELECT` from another
  snapshot (UNIQUE indexes, partial indexes, CHECKs, seed data).

Evidence was gathered by 10 parallel forensic passes over the live DB (`pg_dump`,
`pg_sequences`, `pg_inherits`, `pg_constraint`), the migrations under
`crates/db/migrations/`, the indexer write paths in
`crates/indexer/src/handler/persist/write.rs`, and the runner in
`crates/backfill-runner/src/`.

---

## Decision

Adopt the merge strategy below. **Do not write a "naive UNION" merge script.**
A correct merge requires per-table semantics, surrogate-id remapping for four
tables (`accounts`, `soroban_contracts`, `nfts`, `transactions`),
natural-key dedup for three more (`assets`, `liquidity_pool_snapshots`,
`operations_appearances`), watermark-based reconciliation for current-state
tables, and seed-aware deduplication.

### Table-by-table merge semantics

| Table                                           | Write pattern                                                                                                                       | Merge strategy                                                                                                                                                                               |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `_sqlx_migrations`                              | append                                                                                                                              | **Precondition gate** — both DBs must show the identical successful migration set                                                                                                            |
| `ledgers`                                       | `ON CONFLICT (sequence) DO NOTHING`                                                                                                 | UNION; safe — natural PK, ranges disjoint                                                                                                                                                    |
| `transaction_hash_index`                        | `ON CONFLICT (hash) DO NOTHING`                                                                                                     | UNION; safe — natural PK                                                                                                                                                                     |
| `wasm_interface_metadata`                       | `ON CONFLICT (wasm_hash) DO UPDATE`                                                                                                 | UNION with `DO UPDATE SET metadata = EXCLUDED.metadata` (newer non-empty wins)                                                                                                               |
| `liquidity_pools`                               | `ON CONFLICT (pool_id) DO UPDATE` (LEAST `created_at_ledger`)                                                                       | UNION with same merge clause                                                                                                                                                                 |
| `accounts`                                      | upsert by `account_id`; surrogate `id BIGSERIAL`                                                                                    | **Remap `id`**; merge by natural key with `LEAST(first_seen_ledger)`, `GREATEST(last_seen_ledger)`, conditional `sequence_number` ignoring sentinel `-1`, latest non-NULL `home_domain`      |
| `soroban_contracts`                             | upsert by `contract_id`; surrogate `id BIGSERIAL`                                                                                   | **Remap `id`**; merge with `COALESCE` per column, `is_sac = OR` (monotonic)                                                                                                                  |
| `assets`                                        | 4 upsert paths by `asset_type`; surrogate `id SERIAL`                                                                               | **Dedup-only** (no FK referrers); merge by partial UNIQUEs (`uidx_assets_native`, `uidx_assets_classic_asset`, `uidx_assets_soroban`); preserve the `GREATEST(asset_type)` monotonic upgrade |
| `nfts`                                          | upsert by `(contract_id, token_id)`; surrogate `id SERIAL`; LWW `current_owner_*` by `current_owner_ledger`                         | **Remap `id`**; merge by natural key; **rebuild** `current_owner_id` / `current_owner_ledger` from merged `nft_ownership`                                                                    |
| `transactions` (partitioned)                    | `ON CONFLICT uq_transactions_hash_created_at DO UPDATE` (no-op for `RETURNING id`); surrogate `id BIGSERIAL`                        | **Remap `id`**; merge by `(hash, created_at)`                                                                                                                                                |
| `operations_appearances` (partitioned)          | `ON CONFLICT uq_ops_app_identity DO NOTHING` (NULLS NOT DISTINCT, wide natural key); surrogate `id BIGSERIAL`                       | **Dedup-only** (no FK referrers); rely on the wide UNIQUE                                                                                                                                    |
| `liquidity_pool_snapshots` (partitioned)        | `ON CONFLICT uq_lp_snapshots_pool_ledger DO NOTHING`; surrogate `id BIGSERIAL`                                                      | **Dedup-only** (no FK referrers); rely on `uq_lp_snapshots_pool_ledger`                                                                                                                      |
| `transaction_participants` (partitioned)        | `ON CONFLICT (account_id, created_at, transaction_id) DO NOTHING`                                                                   | UNION after FK remap — PK is the natural key                                                                                                                                                 |
| `soroban_events_appearances` (partitioned)      | `ON CONFLICT (contract_id, transaction_id, ledger_sequence, created_at) DO NOTHING`                                                 | UNION after FK remap — PK is the natural key                                                                                                                                                 |
| `soroban_invocations_appearances` (partitioned) | `ON CONFLICT (contract_id, transaction_id, ledger_sequence, created_at) DO NOTHING`; XOR-checked `caller_id` / `caller_contract_id` | UNION after FK remap                                                                                                                                                                         |
| `nft_ownership` (partitioned)                   | `ON CONFLICT (nft_id, created_at, ledger_sequence, event_order) DO NOTHING` (full event log)                                        | UNION after FK remap                                                                                                                                                                         |
| `lp_positions`                                  | upsert by `(pool_id, account_id)` watermark `last_updated_ledger`                                                                   | **Watermark-aware** UPSERT: `shares` only overwritten when `EXCLUDED.last_updated_ledger ≥ existing`; `LEAST(first_deposit_ledger)`, `GREATEST(last_updated_ledger)`                         |
| `account_balances_current`                      | upsert with watermark `last_updated_ledger` (per ADR 0035 — current only, history dropped)                                          | **Watermark-aware** UPSERT identical pattern, separate paths for native vs credit                                                                                                            |

### Required pre-merge invariants

1. **Identical migration baseline.** On every snapshot:
   `SELECT version, success FROM _sqlx_migrations ORDER BY version;` must match
   exactly across all laptops. Today that is 16 rows ending at
   `20260430000000_invocations_caller_contract`.
2. **Disjoint ledger ranges.** `MIN/MAX(sequence)` from `ledgers` per snapshot
   must not overlap. If they do, switch to `DO NOTHING` for append tables and
   rebuild current-state tables from history.
3. **Same partition layout.** All partitioned tables currently route into
   `*_default` only — there are no explicit RANGE children. Verify identical
   on every laptop before merge; if any laptop ran `db-partition-mgmt` and
   created monthly children, the target must have matching children before
   `ATTACH PARTITION` or before plain `INSERT … SELECT` from the source's
   `*_default`.
4. **Same schema constraint set.** Confirm `ck_assets_identity` (ADR 0038),
   `ck_sia_caller_xor`, and the partial UNIQUEs on `assets` and
   `account_balances_current` are present on every snapshot.

### Surrogate-id remap procedure (four tables with FK referrers + three dedup-only)

Seven sequences exist. **Four** own surrogate keys that are **referenced** by
FKs elsewhere and therefore require a full remap-with-FK-rewrite pass when
merging snapshot N (N ≥ 2) into snapshot 1: `accounts`, `soroban_contracts`,
`nfts`, `transactions`. The other three (`assets`, `liquidity_pool_snapshots`,
`operations_appearances`) have no FK referrers — their surrogate ids still
allocate from per-laptop sequences and therefore still collide on insert, but
the fix is natural-key dedup via existing UNIQUE constraints, not FK
rewriting. All seven are listed below for completeness:

| Sequence                          | Owner                         | Referenced by                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `accounts_id_seq`                 | `accounts.id`                 | `transactions.source_id`, 3 cols on `operations_appearances`, `transaction_participants.account_id`, `soroban_contracts.deployer_id`, `soroban_invocations_appearances.caller_id`, `assets.issuer_id`, `nfts.current_owner_id`, `nft_ownership.owner_id`, `liquidity_pools.asset_a/b_issuer_id`, `lp_positions.account_id`, `account_balances_current.account_id` + `issuer_id` |
| `soroban_contracts_id_seq`        | `soroban_contracts.id`        | `assets.contract_id`, `nfts.contract_id`, `operations_appearances.contract_id`, `soroban_events_appearances.contract_id`, `soroban_invocations_appearances.contract_id` + `caller_contract_id`                                                                                                                                                                                  |
| `assets_id_seq`                   | `assets.id`                   | (no FK references today; remap-safe but still must dedup)                                                                                                                                                                                                                                                                                                                       |
| `nfts_id_seq`                     | `nfts.id`                     | `nft_ownership.nft_id` (CASCADE)                                                                                                                                                                                                                                                                                                                                                |
| `transactions_id_seq`             | `transactions.id`             | `(transaction_id, created_at)` on the four partitioned appearance tables (CASCADE), `transaction_participants`                                                                                                                                                                                                                                                                  |
| `liquidity_pool_snapshots_id_seq` | `liquidity_pool_snapshots.id` | (no FK references)                                                                                                                                                                                                                                                                                                                                                              |
| `operations_appearances_id_seq`   | `operations_appearances.id`   | (no FK references)                                                                                                                                                                                                                                                                                                                                                              |

For the four tables with FK referrers, in topological order
(`accounts` → `soroban_contracts` → `nfts` → `transactions`; `soroban_contracts`
must precede `nfts` because `nfts.contract_id` FKs `soroban_contracts.id`):

1. Build a temp table mapping `(natural_key) → (source_id, target_id)` by
   joining source rows against existing target rows, then assigning fresh ids
   (let target's BIGSERIAL allocate via `INSERT … RETURNING`) for newcomers.
2. `INSERT INTO target (…) SELECT … ON CONFLICT (natural_key) DO UPDATE …`
   using each table's existing upsert clause, capturing
   `RETURNING id, natural_key` to learn the target id for every source row.
3. For every dependent table, rewrite the FK column via the map before insert
   (do this in the SELECT, never as a post-insert UPDATE — the partitioned
   tables are huge).

**Full topological merge order** (verified against live DB FK graph, not just
the four sequences above): `_sqlx_migrations` (precondition) → `ledgers` →
`accounts` → `wasm_interface_metadata` → `soroban_contracts` (FKs:
`deployer_id` → accounts, `wasm_hash` → wasm_interface_metadata) → `assets`
(FKs: `issuer_id` → accounts, `contract_id` → soroban_contracts) →
`liquidity_pools` (FKs: `asset_a/b_issuer_id` → accounts) → `nfts` (FKs:
`contract_id` → soroban_contracts, `current_owner_id` → accounts) →
`transactions` (FK: `source_id` → accounts) → `transaction_hash_index` (no
FKs) → all five appearance tables (`operations_appearances` also FKs
`pool_id` → liquidity_pools, so liquidity_pools must precede it) →
`liquidity_pool_snapshots`, `lp_positions`, `account_balances_current`.

For the three dedup-only tables (`assets`, `liquidity_pool_snapshots`,
`operations_appearances`), `INSERT … SELECT … ON CONFLICT … DO NOTHING/UPDATE`
on the existing natural-key UNIQUEs (partial indexes on `assets`,
`uq_lp_snapshots_pool_ledger`, `uq_ops_app_identity`) is sufficient. The
surrogate id of each row will be whatever the target sequence allocates;
because no other table FKs into them, that is fine.

After all merges complete, run
`SELECT setval('<seq>', (SELECT MAX(id) FROM <table>));` for **every** sequence
(all seven) so future inserts on the merged DB don't collide.

### Watermark merges (current-state tables)

`lp_positions`, `account_balances_current`, and `nfts.current_owner_*` are
**Last-Writer-Wins by ledger_sequence**. A naive `INSERT … ON CONFLICT
DO UPDATE SET balance = EXCLUDED.balance` will pick whichever snapshot loaded
last, not whichever has the freshest ledger. Replicate the indexer's clause:

```sql
ON CONFLICT (<natural_key>) DO UPDATE SET
  shares = CASE WHEN EXCLUDED.last_updated_ledger >= target.last_updated_ledger
           THEN EXCLUDED.shares ELSE target.shares END,
  last_updated_ledger = GREATEST(target.last_updated_ledger,
                                 EXCLUDED.last_updated_ledger),
  first_deposit_ledger = LEAST(target.first_deposit_ledger,
                               EXCLUDED.first_deposit_ledger)
```

For `nfts.current_owner_id` / `current_owner_ledger`: do **not** trust either
snapshot's cached values — rebuild after the merge by selecting, per
`(contract_id, token_id)`, the row with `MAX(ledger_sequence, event_order)`
from the merged `nft_ownership`.

### Cross-range dangling references — the hardest problem

`backfill-runner` does **no** cross-ledger lookups. Each laptop parses XDR and
inserts rows in isolation. So for the same logical entity (e.g. account
`G…ABC`), laptop A and laptop B both run their own `INSERT … ON CONFLICT
(account_id) …` and end up with **different surrogate `id` values for the same
StrKey**. This is what forces the remap.

It also means that if laptop B's range has an event referencing a contract
that was _deployed_ in laptop A's range, B will create its own bare
`soroban_contracts` row (the upsert path inserts a stub by `contract_id`,
filling `wasm_hash` / `deployer_id` / `is_sac` lazily). The merge's
`COALESCE`-based union of contract rows is exactly what reconciles this — but
only because the indexer was already designed to tolerate "stub-now,
fill-later" upserts. Confirm before merge that no FK is `NOT NULL` for fields
that may only have been populated on one laptop (today: `deployer_id`,
`wasm_hash`, `contract_type`, `metadata` are all nullable on
`soroban_contracts`; `current_owner_id` is nullable on `nfts`).

### Seed data deduplication

Migration `20260428000000_seed_native_asset_singleton` inserts one row into
`assets` (`asset_type=0`, name='Stellar Lumen'). It runs on every laptop, so
every snapshot has that row. The partial UNIQUE `uidx_assets_native ON
assets((asset_type)) WHERE asset_type = 0` will reject the second copy at
merge time. Use `ON CONFLICT DO NOTHING` on the assets merge step (or
explicitly skip the native row when copying from non-canonical snapshots).

### Partition handling

Today every partitioned parent has only a `*_default` child. Two consequences:

- `INSERT … SELECT` from `<source>.<table>_default` into
  `<target>.<table>_default` works without partition gymnastics.
- If a future laptop runs `db-partition-mgmt` and creates monthly children
  while another laptop did not, the merge target must have those children
  too — otherwise rows that fall in a monthly range will be rejected (default
  partition is _exclusive_ of any range covered by an explicit child). Run
  `db-partition-mgmt` on the target to the highest set seen across snapshots
  before merging.

### CASCADE traps

`transactions` is the apex of five `ON DELETE CASCADE` chains
(`operations_appearances`, `transaction_participants`, `soroban_events_appearances`,
`soroban_invocations_appearances`, `nft_ownership`). During the merge, never
`DELETE FROM transactions` to "clean up duplicates" — that wipes the
appearance rows. Use `ON CONFLICT DO NOTHING/UPDATE` instead.

`nft_ownership` also CASCADEs from `nfts`. Same rule.

**`soroban_contracts.search_vector` is `GENERATED ALWAYS`.** Verified via
`information_schema.columns` — the column is defined as
`tsvector GENERATED ALWAYS AS (to_tsvector('simple',
COALESCE(metadata->>'name','') || ' ' || contract_id)) STORED`. Postgres
forbids explicit values for generated columns on INSERT, so the merge
script's `INSERT INTO soroban_contracts (col_list) SELECT col_list FROM
source` **must omit `search_vector` from `col_list`** (and from any
`SELECT *` shorthand). Postgres will recompute it from the merged `metadata`
and `contract_id` automatically. If you forget, `INSERT … SELECT *` raises
`ERROR: cannot insert a non-DEFAULT value into column "search_vector"`.

**Runtime objects the merge target must already have** (covered by the
"identical migration baseline" precondition, but listed explicitly so a
custom-built target doesn't surprise you): the `pg_trgm` extension (used by
GIN trigram indexes on `assets.asset_code`, `nfts.collection_name`,
`nfts.name`, and `soroban_contracts.search_vector`) and five IMMUTABLE
label functions in `public` — `op_type_name`, `asset_type_name`,
`token_asset_type_name`, `nft_event_type_name`, `contract_type_name` —
created by migration `20260422000000_enum_label_functions`. These are
schema-DDL, not data, so they appear in any `pg_dump --schema-only` of a
laptop that ran the full migration set.

**Postgres partition-FK quirk to expect.** Each of the five appearance-table
_parents_ (and `transaction_participants`, `nft_ownership`) carries **two**
foreign keys to transactions in `pg_constraint`: one to
`transactions(id, created_at)` and one to `transactions_default(id, created_at)`
with a `_fkey1` suffix. Postgres adds the second one automatically when the
default partition is attached. Both are CASCADE and target the same logical
row, so they don't cause double-cascade or duplicate insert validation —
but a merge-script audit query like
`SELECT count(*) FROM pg_constraint WHERE contype='f'` will return roughly
double what you expect. Don't treat this as drift.

---

## Rationale

The schema was designed for a single-writer pipeline with replay-safe upserts.
The replay-safety almost works for cross-DB merging, with two specific holes:

1. **Surrogate keys are DB-local.** `BIGSERIAL` allocates from a per-DB
   sequence; the same natural key gets different surrogate ids on different
   laptops. Four tables expose surrogate ids that are referenced as FKs
   elsewhere (`accounts`, `soroban_contracts`, `nfts`, `transactions`), so
   the merge cannot just `INSERT … SELECT *` — it must rewrite the FK column
   on every dependent row. This is the single biggest source of complexity
   in the merge script and the reason a "naive `pg_dump | pg_restore`"
   approach is wrong.
2. **Current-state tables clobber by load order, not by ledger order.** The
   indexer's upserts already use `last_updated_ledger` watermarks, but a
   merge script that omits them will pick whichever snapshot was loaded last,
   silently corrupting balances and LP positions when ranges overlap or when
   a row from a later range is loaded before a row from an earlier range.

Everything else (append-only tables, partitioned appearance tables with
natural-key UNIQUEs, monotonic columns) merges cleanly under the rules
already encoded in the indexer's `ON CONFLICT` clauses.

---

## Alternatives Considered

### Alternative 1: Single-laptop sequential backfill

**Description:** Skip the parallel approach; one machine runs the full
historical backfill in ledger order.

**Pros:**

- No merge script needed.
- `last_updated_ledger` watermarks behave identically to production, no
  reconciliation step.
- Surrogate-id sequences are globally unique by construction.

**Cons:**

- Linear in wall-clock time — the whole point of using N laptops is to cut
  this by ~N.

**Decision:** REJECTED — the parallelism win is the explicit goal.

### Alternative 2: Logical replication / one shared DB

**Description:** Run a single Postgres on a shared host; every laptop's
`backfill-runner` writes to the same DB.

**Pros:**

- No merge step at all.
- Surrogate-id allocation stays serial.

**Cons:**

- Defeats "local Docker DB per laptop" — needs network round-trip on every
  insert. UNNEST batches help but won't recover the order of magnitude lost.
- WAL contention on hot tables (`accounts`, `soroban_contracts`, `assets`)
  would pin throughput to single-writer levels anyway.

**Decision:** REJECTED — undermines the perf goal.

### Alternative 3: Naive `pg_dump | pg_restore` with `--data-only`

**Description:** Dump each laptop's data and restore one after another into a
fresh target.

**Pros:**

- Trivially simple to run.

**Cons:**

- Surrogate-id collisions on every BIGSERIAL primary key. The second restore
  fails on PK conflict for `accounts`, `soroban_contracts`, `transactions`,
  etc.
- Even if collisions were resolved by offsetting sequences, FKs would still
  point to the wrong rows because the same StrKey gets different ids per
  laptop.
- No watermark logic for current-state tables — naive restore order
  determines `account_balances_current` values.

**Decision:** REJECTED — produces a structurally inconsistent DB.

### Alternative 4: Use natural keys everywhere, drop surrogate ids

**Description:** Refactor the schema so every table uses natural keys (StrKey
for accounts/contracts, hash for transactions). Then merge becomes UNION ALL

- dedup.

**Pros:**

- Merging is trivial.

**Cons:**

- Reverses ADRs 0026 (accounts surrogate) and 0030 (contracts surrogate),
  which were taken specifically because BIGINT joins are dramatically faster
  than VARCHAR(56) joins on the partitioned appearance tables.
- Touches every read path and every index.
- Out of scope for a one-off backfill.

**Decision:** REJECTED — too invasive for a transient merge problem.

---

## Consequences

### Positive

- The merge script has a complete, reviewed map of every conflict target,
  every surrogate-id remap, and every watermark-sensitive column before code
  is written.
- Future readers can answer "what does this table do at merge time?" from a
  single document instead of re-reading 14 INSERT sites in `write.rs`.
- The required pre-merge invariants (identical migration set, disjoint
  ledger ranges, identical partition layout) are stated explicitly so the
  script can assert them at start instead of failing late.

### Negative

- The merge script is non-trivial — at minimum four remap passes (`accounts`,
  `soroban_contracts`, `nfts`, `transactions`) with FK rewrites in every
  dependent SELECT, plus natural-key dedup passes for `assets`,
  `liquidity_pool_snapshots`, `operations_appearances`. Plan for it
  accordingly.
- A correctness regression in the indexer's ON CONFLICT clauses now has a
  second blast radius (the merge script depends on them being right).
- `nfts.current_owner_*` reconciliation requires a post-merge scan over the
  full `nft_ownership` event log — measurable cost on full-history merges.

---

## Delivery Checklist

Per [ADR 0032](./0032_docs-architecture-evergreen-maintenance.md):

- [ ] `docs/architecture/technical-design-general-overview.md` — N/A (no
      architectural shape change; one-off operational procedure)
- [ ] `docs/architecture/database-schema/database-schema-overview.md` — N/A
      (schema unchanged)
- [ ] `docs/architecture/backend/backend-overview.md` — N/A
- [ ] `docs/architecture/frontend/frontend-overview.md` — N/A
- [ ] `docs/architecture/indexing-pipeline/indexing-pipeline-overview.md` —
      N/A (backfill-runner and indexer behaviour unchanged; ADR documents a
      planned offline merge procedure, not a pipeline change)
- [ ] `docs/architecture/infrastructure/infrastructure-overview.md` — N/A
- [ ] `docs/architecture/xdr-parsing/xdr-parsing-overview.md` — N/A
- [ ] This ADR is linked from each updated doc at the relevant section — N/A
      (no doc updates triggered)

This is a **policy / procedure ADR** — it captures planning context for a
future merge script and does not alter the shape of the system. If the merge
script becomes a permanent crate (e.g. `crates/db-merge`), revisit and update
the indexing-pipeline doc.

---

## References

- `crates/db/migrations/` — every migration up to
  `20260430000000_invocations_caller_contract`
- `crates/indexer/src/handler/persist/write.rs` — every INSERT / UPSERT site
  surveyed (ledgers, transactions, accounts, soroban_contracts, assets, nfts,
  nft_ownership, operations_appearances, transaction_participants,
  soroban_events_appearances, soroban_invocations_appearances,
  liquidity_pools, liquidity_pool_snapshots, lp_positions,
  account_balances_current, wasm_interface_metadata, transaction_hash_index)
- `crates/backfill-runner/src/{run,ingest,resume,partition,sync}.rs` —
  range parameterisation, idempotency, partition assumptions
- `crates/db-partition-mgmt/` — partition naming and `_default` handling
- ADR 0026 — accounts surrogate id rationale
- ADR 0030 — contracts surrogate id rationale
- ADR 0035 — drop account balance history (current-state only)
- ADR 0037 — current schema snapshot
- ADR 0038 — `ck_assets_identity` loosening for native XLM SAC
