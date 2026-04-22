---
id: '0152'
title: 'Implement ADR 0031: enum-like VARCHAR columns → SMALLINT + Rust enum'
type: FEATURE
status: active
related_adr: ['0027', '0030', '0031']
related_tasks: ['0149', '0151']
tags:
  [
    layer-backend,
    layer-indexer,
    layer-db,
    layer-api,
    priority-medium,
    effort-large,
    adr-0031,
    schema-migration,
    storage,
    performance,
  ]
links:
  - lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md
  - crates/db/migrations
  - crates/indexer/src/handler/persist/write.rs
  - crates/xdr-parser/src
history:
  - date: '2026-04-21'
    status: backlog
    who: fmazur
    note: >
      Spawned from 0151 future work. ADR 0031 drafted during 0151 review
      identified ~160-220 GB/year saving + ~2-3× faster WHERE type=… probes
      by flipping 7-8 enum-like VARCHAR columns to SMALLINT + Rust
      #[repr(i16)] enum + CHECK range. Preconditions: ADR 0030 must be
      landed (done via 0151).
  - date: '2026-04-21'
    status: active
    who: fmazur
    note: >
      Activating right after closing 0151 + 0149. Set as current task.
      Implementation order per ADR 0031: schema migrations 0002-0007 in
      place (source-of-truth, per 0151 precedent), Rust #[repr(i16)] enum
      modules per column, persist binds flipped String→i16, integration
      test enumerating every variant against op_type_name SQL helper.
---

# Implement ADR 0031: enum-like VARCHAR columns → `SMALLINT` + Rust enum

## Summary

Apply ADR 0031 design: every enum-like `VARCHAR(N)` column (closed
protocol-defined domain) becomes `SMALLINT NOT NULL` guarded by a
`CHECK` range. Rust `#[repr(i16)]` enum in `crates/domain/src/enums/`
is the single source of truth for each mapping. API serializes to
canonical string via serde; ad-hoc SQL uses `IMMUTABLE` helper
functions for readable labels in psql/BI.

## Context

ADR 0031 proposed during task 0151 review. Census of current data
(100-ledger bench) showed ~2.7 MB heap + ~0.7 MB indexes spent on
enum-like VARCHAR columns — extrapolates to ~160-220 GB/year at
mainnet scale. Implementation preconditions met post-0151:
source-of-truth migrations 0002-0005 already rewritten with
ADR 0030 shape; this task adds the SMALLINT flip on top.

## Implementation

### Phase 1 — schema migration

Edit source-of-truth migrations (same pattern as 0151):

- `0003_transactions_and_operations.sql` — `operations.type`
  `VARCHAR(50)` → `SMALLINT NOT NULL` + `ck_ops_type_range CHECK
(type BETWEEN 0 AND 127)`
- `0004_soroban_activity.sql` — `soroban_events.event_type` →
  `SMALLINT NOT NULL` + `CHECK (event_type BETWEEN 0 AND 15)`
- `0005_tokens_nfts.sql` — `tokens.asset_type`,
  `nft_ownership.event_type` → SMALLINT + CHECK
- `0006_liquidity_pools.sql` — `liquidity_pools.asset_a_type`,
  `asset_b_type` → SMALLINT + CHECK
- `0007_account_balances.sql` — `account_balances_current.asset_type`,
  `account_balance_history.asset_type` → SMALLINT + CHECK
- `0002_identity_and_ledgers.sql` — `soroban_contracts.contract_type`
  → SMALLINT + CHECK

New migration `00XX_enum_label_functions.sql` ships IMMUTABLE helper
functions per enum: `op_type_name(SMALLINT) RETURNS TEXT`,
`asset_type_name`, `event_type_name`, `contract_type_name`,
`nft_event_type_name`. Each is a simple `CASE WHEN` expression; planner
inlines.

### Phase 2 — Rust domain enums + persist

New crate module `crates/domain/src/enums/` (or `crates/xdr-parser/src/enums/`
if closer to XDR source), one file per enum:

- `operation_type.rs` — `OperationType` with 27 variants (Stellar
  Protocol 21). `#[derive(sqlx::Type, Serialize, Deserialize, ToSchema)]`
  `#[repr(i16)]`. `as_str()` returns canonical label.
- `asset_type.rs` — `AssetType` (XDR 4 variants).
- `token_asset_type.rs` — explorer-synthetic 4-variant
  `{native, classic, sac, soroban}`.
- `contract_event_type.rs` — `SYSTEM/CONTRACT/DIAGNOSTIC`.
- `nft_event_type.rs` — parser-internal.
- `contract_type.rs` — explorer-synthetic.

Refactor parser in `crates/xdr-parser/src/operation.rs` etc. to emit
typed enum (skip the `Debug`/`Display` string round-trip). Persist
layer (`crates/indexer/src/handler/persist/write.rs`) flips affected
`.bind(…_vec: Vec<String>)` to `.bind(…_vec: Vec<OperationType>)` (or
`Vec<i16>` after explicit `as i16` cast, depending on sqlx encoder
shape).

### Phase 3 — Integration test

Extend `persist_integration.rs` to verify round-trip for at least one
enum column (e.g. `operations.type`): insert → fetch by Rust enum
compare → assert equality. One `for v in OperationType::VARIANTS`
iteration verifying `op_type_name(v as i16) = v.as_str()` closes the
Rust ↔ SQL drift gap.

### Phase 4 — API enum serde

When backend module tasks (0046 transactions, 0049 tokens, etc.)
resume, each handler decodes SMALLINT → Rust enum → serde emits the
canonical label. Zero JOIN anywhere (unlike ADR 0030 which needed
`soroban_contracts` JOIN on display).

## Acceptance Criteria

- [ ] Migrations 0002-0007 updated in place; `npm run db:reset` passes.
- [ ] Every enum column has a paired `CHECK` range constraint.
- [ ] Each Rust enum derives `sqlx::Type`, `Serialize`, `Deserialize`,
      `ToSchema`; `#[repr(i16)]` pins on-disk layout.
- [ ] `persist_integration.rs` round-trip test for at least one enum
      column passes.
- [ ] New integration test iterating every variant: `op_type_name(v as i16)
  == v.as_str()` for all, and same for other enums.
- [ ] `backfill-bench --start 62016000 --end 62016099` indexes 100
      ledgers without errors; p95 measured, expected improvement on
      `WHERE type = …` filter probes on partitions > ~10 k rows.
- [ ] DB size after 100 ledgers compared to post-0030 baseline (this
      task's starting point). Capture per-table delta.
- [ ] `cargo clippy --all-targets -- -D warnings` green.
- [ ] `SQLX_OFFLINE=true cargo build --workspace` green.
- [ ] ADR 0031 promoted to `accepted` after landing.

## Out of Scope

- `asset_code VARCHAR(12)` — open domain (issuer-defined), doesn't fit
  the enum pattern. Explicitly scoped out in ADR 0031 §5.
- `function_name VARCHAR(100)` on `soroban_invocations` — arbitrary
  per-contract Soroban symbol.
- Any column ordering re-layout beyond the one-liner alignment nudge
  in ADR 0031 §4 (which happens naturally during each `ALTER COLUMN`
  pass).

## Notes

- **Helper function discipline**: an integration test MUST enumerate
  every `#[repr(i16)]` variant and compare `enum::as_str()` against
  the SQL helper function's output. Silent drift between the two
  (e.g. adding a Rust variant without updating `op_type_name`) would
  return NULL from the function — catch it in tests, not in prod.
- **Source-of-truth migrations vs. new migration**: follow 0151
  precedent — edit existing `0002-0007` files in place; no new
  timestamped `yyyymmdd_enum_columns_smallint.sql`. Project pre-GA.
