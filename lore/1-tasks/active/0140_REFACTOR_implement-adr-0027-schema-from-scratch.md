---
id: '0140'
title: 'DB: implement ADR 0027 schema from scratch (wipe existing migrations)'
type: REFACTOR
status: active
related_adr:
  [
    '0011',
    '0012',
    '0013',
    '0014',
    '0015',
    '0016',
    '0017',
    '0018',
    '0019',
    '0020',
    '0021',
    '0022',
    '0023',
    '0024',
    '0025',
    '0026',
    '0027',
  ]
related_tasks: ['0136']
tags: [priority-high, effort-large, layer-db, schema]
links:
  - crates/db/migrations/
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
history:
  - date: '2026-04-20'
    status: backlog
    who: fmazur
    note: 'Task created — implement ADR 0027 as the initial schema. Delete existing migrations 0001-0009, write clean migrations producing the post-surrogate shape directly. Scope limited to DDL only; Rust/API updates are separate follow-ups.'
  - date: '2026-04-20'
    status: active
    who: fmazur
    note: 'Activated task — promoted from backlog to active, set as current task.'
---

# DB: implement ADR 0027 schema from scratch (wipe existing migrations)

## Summary

Implement the schema defined by ADR 0027 (post-surrogate snapshot) as the project's
initial schema. Existing migrations `0001–0009` reflect the pre-ADR-0011..0026 shape
and are now obsolete — delete them and write clean migrations that produce the ADR 0027
target directly. No incremental ALTERs, no data preservation logic.

Scope is limited to **DDL only** (tables, indexes, constraints, partitioning). Rust
persistence code, query updates, and API JOIN adjustments are separate follow-up tasks.

## Context

ADR 0027 is the authoritative schema snapshot after the ADR 0011–0026 iteration chain.
It defines 18 logical tables with:

- `accounts.id BIGSERIAL` surrogate PK (ADR 0026)
- `BYTEA(32)` hashes (ADR 0024)
- Typed token metadata columns (ADR 0023)
- Monthly partitioning on time-series tables
- Partial UNIQUE indexes for native-XLM balance rows
- Full endpoint realizability across all 22 API endpoints

The current migrations (`0001_create_ledgers_transactions.sql` … `0009_wasm_interface_metadata_staging.sql`)
are pre-ADR shape — VARCHAR account FKs, hash columns not yet BYTEA, no surrogate IDs.
There is no production data yet that must be preserved, so rewriting migrations from
scratch is cheaper and cleaner than producing a long chain of ALTER migrations to reach
the ADR 0027 target.

**Supersedes task 0136** — that task proposed incremental surrogate-ID migration. With
no production data, a clean rewrite is simpler. 0136 should be closed as superseded once
this task lands.

## Implementation Plan

### Step 1: Wipe existing migrations

Move all 9 existing migrations to `.trash/` (project policy forbids `rm`):

```
crates/db/migrations/
  0001_create_ledgers_transactions.sql   → .trash/
  0002_create_operations.sql             → .trash/
  0003_create_soroban_contracts.sql      → .trash/
  0004_create_soroban_activity_tables.sql → .trash/
  0005_create_accounts_tokens.sql        → .trash/
  0006_create_nfts_pools_snapshots.sql   → .trash/
  0007_idempotent_write_constraints.sql  → .trash/
  0008_index_contracts_wasm_hash.sql     → .trash/
  0009_wasm_interface_metadata_staging.sql → .trash/
```

Also check `.sqlx/` offline query cache — any entry tied to removed/changed queries
must be regenerated after schema rewrite (`npm run db:prepare`).

### Step 2: Write clean migrations for ADR 0027 shape

Split the 18 tables across a small set of ordered migrations, respecting FK dependency
order. Proposed layout:

| #    | File                                   | Tables                                                                                                                       |
| ---- | -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- |
| 0001 | `0001_extensions.sql`                  | `CREATE EXTENSION pg_trgm` (for GIN trigram indexes on tokens/nfts)                                                          |
| 0002 | `0002_identity_and_ledgers.sql`        | `ledgers`, `accounts`, `wasm_interface_metadata`, `soroban_contracts`                                                        |
| 0003 | `0003_transactions_and_operations.sql` | `transactions` (partitioned), `transaction_hash_index`, `operations` (partitioned), `transaction_participants` (partitioned) |
| 0004 | `0004_soroban_activity.sql`            | `soroban_events` (partitioned), `soroban_invocations` (partitioned)                                                          |
| 0005 | `0005_tokens_nfts.sql`                 | `tokens`, `nfts`, `nft_ownership` (partitioned)                                                                              |
| 0006 | `0006_liquidity_pools.sql`             | `liquidity_pools`, `liquidity_pool_snapshots` (partitioned), `lp_positions`                                                  |
| 0007 | `0007_account_balances.sql`            | `account_balances_current`, `account_balance_history` (partitioned)                                                          |

Copy DDL verbatim from ADR 0027 Part I. Preserve:

- Composite PKs on partitioned tables (`(id, created_at)`)
- Partial UNIQUE indexes for native-XLM rows (ADR 0027 §17, §18)
- CHECK constraints (hash lengths, asset-type ↔ null-issuer symmetry, pool_id length)
- Partial indexes with `WHERE` clauses (e.g. `idx_events_transfer_from WHERE transfer_from_id IS NOT NULL`)
- Generated column `soroban_contracts.search_vector TSVECTOR GENERATED ALWAYS AS (...) STORED`

Partition setup: the `CREATE TABLE ... PARTITION BY RANGE (created_at)` lines create the
parent only. Initial monthly partitions for the current month are likely still owned by
the partition Lambda (see task 0139) — check whether bootstrap partitions need to be part
of this migration or remain Lambda-managed.

### Step 3: Update `.sqlx/` offline cache + verify build

```bash
npm run db:reset    # drop + recreate + apply all migrations
npm run db:prepare  # regenerate .sqlx/ for offline CI
```

Any `sqlx::query!()` call in `crates/db/src/` that still references removed columns
(e.g. `source_account VARCHAR`) will fail to compile. Those failures are expected — they
mark the interface surface that the Rust follow-up task must update. This task fixes them
only if the fix is a trivial rename (e.g. `source_account` → `source_id`), otherwise
files the failure under the follow-up task.

### Step 4: Spawn follow-up task(s)

After this task lands, spawn:

- **Rust persistence/query updates** — rewrite inserts, selects, and joins in
  `crates/db/src/persistence.rs` + `soroban.rs` to use surrogate IDs, BYTEA hashes,
  typed token metadata columns. Includes StrKey → `accounts.id` resolver.
- **API layer updates** — per ADR 0027 Part III, endpoints now JOIN `accounts` for
  StrKey display and need a StrKey resolver on route-param intake.

Close **0136** as superseded in the same PR (its surrogate-ID work is absorbed here).

## Acceptance Criteria

- [ ] Existing `crates/db/migrations/0001–0009` moved to `.trash/`
- [ ] New migrations produce the exact ADR 0027 schema (18 tables, all indexes, all
      CHECK constraints, partial UNIQUE indexes, generated columns, partitioning)
- [ ] `pg_trgm` extension created before any GIN trigram index
- [ ] FK dependency order respected across migration files
- [ ] `npm run db:reset` succeeds on a clean database
- [ ] `npm run db:prepare` runs to completion; `.sqlx/` regenerated and committed
- [ ] Cargo build status documented — either green (trivial renames applied) or the
      list of failing `sqlx::query!()` sites is handed to the follow-up task
- [ ] Task 0136 marked superseded (history entry + move to archive)
- [ ] Follow-up tasks spawned for Rust persistence + API layer updates
- [ ] Decision on bootstrap partitions (this migration vs Lambda) documented

## Notes

- No data-migration logic required — wipe-and-recreate is acceptable because there is no
  production data to preserve. If that assumption changes before this task starts, revisit.
- `MIGRATIONS.md` says migrations 0001–0004 are "irreversible (initial schema, never
  revert)" — this task resets that marker to the new 0001–0007. Update `MIGRATIONS.md`
  accordingly.
- ADR 0027 is `proposed`, not `accepted`. Activating this task should coincide with
  promoting ADR 0027 to `accepted`, since implementing it is the acceptance signal.
