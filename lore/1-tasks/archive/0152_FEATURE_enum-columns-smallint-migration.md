---
id: '0152'
title: 'Implement ADR 0031: enum-like VARCHAR columns → SMALLINT + Rust enum'
type: FEATURE
status: completed
related_adr: ['0027', '0030', '0031']
related_tasks: ['0149', '0151', '0153']
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
  - date: '2026-04-22'
    status: completed
    who: fmazur
    note: >
      Landed. 9/10 acceptance criteria met in-code; last criterion
      (ADR 0031 → accepted) done at task close. 6 migrations edited
      in place as source-of-truth (0002-0007) + new 0008 enum label
      functions. 6 Rust `#[repr(i16)]` enums in crates/domain/src/enums/
      with feature-gated sqlx/utoipa derives (parser stays sqlx-free).
      Parser types.rs + operation.rs + event.rs + state.rs + nft.rs
      flipped to typed enums — no more Debug/Display string round-trip.
      Persist layer: 6 bind sites flipped Vec<String>→Vec<EnumT>,
      9 SQL literal string comparisons ('native'/'classic'/'sac'/...)
      rewritten to ordinals. Integration tests: round-trip on
      operations.type + drift test iterating all 43 variants × 6 enums
      vs xxx_name() SQL helpers. Bench 100 ledgers clean (p95=384 ms —
      matches post-0030 baseline 309-385 ms, no regression). DB size
      81 MB vs 84 MB baseline = 3.6% saved at 100-ledger scale
      (~190 GB/year mainnet extrapolation, within ADR forecast of
      160-220 GB/year). Clippy + workspace build green. ADR 0031 →
      accepted. Spawned 0153 for wasm_hash FK violation bug found
      at 1000-ledger bench (unrelated to 0152 — pre-existing FK from
      0151 surfacing on wider backfill window).
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

- [x] Migrations 0002-0007 updated in place; `npm run db:reset` passes.
      10/10 migrations applied clean from fresh Docker volume.
- [x] Every enum column has a paired `CHECK` range constraint. 9 `CHECK`
      constraints added: `ck_sc_contract_type_range`, `ck_ops_type_range`,
      `ck_events_type_range`, `ck_tokens_asset_type_range`,
      `ck_nft_own_event_type_range`, `ck_lp_asset_a_type_range`,
      `ck_lp_asset_b_type_range`, `ck_abc_asset_type_range`,
      `ck_abh_asset_type_range`.
- [x] Each Rust enum derives `sqlx::Type`, `Serialize`, `Deserialize`,
      `ToSchema`; `#[repr(i16)]` pins on-disk layout. `sqlx`/`utoipa`
      feature-gated in `domain/Cargo.toml` so xdr-parser (which has no DB
      or HTTP surface) doesn't pull them.
- [x] `persist_integration.rs` round-trip test for at least one enum
      column passes. `synthetic_ledger_insert_and_replay_is_idempotent`
      extended: fetches `operations.type` as `OperationType` enum +
      `op_type_name()` label, asserts match on Payment and InvokeHostFunction.
- [x] New integration test iterating every variant: `op_type_name(v as i16)
  == v.as_str()` for all, and same for other enums.
      `enum_label_helpers_match_rust_as_str` iterates 27 + 4 + 4 + 3 + 3 +
      2 = **43 variants × 6 enums** vs all 6 SQL helpers — passes.
- [x] `backfill-bench --start 62016000 --end 62016099` indexes 100
      ledgers without errors; p95 measured. 100/100 indexed, 0 errors;
      min 135 / mean 267 / **p95 384 ms** / p99 414 ms / max 528 ms —
      matches post-0030 baseline (309-385 ms). Filter-probe improvement
      on SMALLINT vs VARCHAR indexes holds up on partitions > 10k rows
      (theoretical 2-3×) but local DB (~80 MB) is entirely in shared
      buffers so micro-benchmark would be noise-dominated; real win
      manifests at mainnet scale.
- [x] DB size after 100 ledgers compared to post-0030 baseline.
      Total **81 MB vs 84 MB** baseline = **3.6% saved**. Per-table:
      `operations` 17→16 MB (-1 MB, saves ~14 B/row × 77k rows),
      `account_balance_history` 11→10 MB (-1 MB), `soroban_events`
      9.8→9.4 MB (-0.4 MB). Mainnet-year extrapolation: ~190 GB/year
      saving — within ADR 0031 forecast of 160-220 GB/year.
- [x] `cargo clippy --all-targets -- -D warnings` green.
- [x] `SQLX_OFFLINE=true cargo build --workspace` green (8 crates).
- [x] ADR 0031 promoted to `accepted` after landing. Done as part of
      task close.

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

## Implementation Notes

### Phase 1 — schema migrations (source-of-truth, edited in place)

- `0002_identity_and_ledgers.sql` — `soroban_contracts.contract_type
SMALLINT` (kept **nullable** — pass-2 bare-row upsert in
  `persist/write.rs` registers referenced contracts before their
  deployment meta is observed; stays NULL until a deploy lands) +
  `ck_sc_contract_type_range CHECK (contract_type IS NULL OR
contract_type BETWEEN 0 AND 15)`.
- `0003_transactions_and_operations.sql` — `operations.type SMALLINT
NOT NULL` + `ck_ops_type_range CHECK (type BETWEEN 0 AND 127)`
  (room beyond Protocol 21's 27 variants).
- `0004_soroban_activity.sql` — `soroban_events.event_type SMALLINT
NOT NULL` + CHECK 0..15.
- `0005_tokens_nfts.sql` — `tokens.asset_type SMALLINT NOT NULL` +
  `ck_tokens_identity` CHECK rewritten from string literals to
  ordinals (`asset_type = 0` native etc.) with inline comments.
  Three partial UNIQUE indexes (`uidx_tokens_native`,
  `uidx_tokens_classic_asset`, `uidx_tokens_soroban`) flipped to
  ordinal predicates (`WHERE asset_type = 0`,
  `WHERE asset_type IN (1, 2)`, `WHERE asset_type IN (2, 3)`).
  `nft_ownership.event_type SMALLINT NOT NULL` + CHECK.
- `0006_liquidity_pools.sql` — `asset_a_type` / `asset_b_type
SMALLINT NOT NULL` + CHECK per column.
- `0007_account_balances.sql` — `asset_type SMALLINT NOT NULL` on
  both tables. `ck_abc_native` / `ck_abh_native` rewritten to
  `asset_type = 0` / `<> 0`. Partial UNIQUE indexes (native vs
  credit) same flip.
- `0008_enum_label_functions.sql` — **new** file. 6 `IMMUTABLE
PARALLEL SAFE` SQL functions: `op_type_name`, `asset_type_name`
  (XDR 4-variant), `token_asset_type_name` (explorer-synthetic),
  `event_type_name`, `nft_event_type_name`, `contract_type_name`.

### Phase 2 — Rust `domain/enums/` module

- 6 files, one per enum, all `#[derive(Debug, Clone, Copy, PartialEq,
Eq, Hash, Serialize, Deserialize)] #[repr(i16)]` +
  `#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]` +
  `#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]`.
- Each enum exposes `pub const VARIANTS: &'static [Self]`,
  `pub const fn as_str(self) -> &'static str`, `impl TryFrom<i16>`,
  `impl FromStr`, `impl Display`. `TryFrom` + `FromStr` both return
  `EnumDecodeError` (typed thiserror) so sqlx decode + API parse
  fail with consistent shape.
- Canonical labels match ADR 0030/0031 wording (SCREAMING_SNAKE_CASE
  for OperationType via `#[serde(rename_all)]`, lowercase for
  lowercase enums, explicit per-variant `#[serde(rename)]` for XDR
  AssetType because snake_case derivation of `CreditAlphanum4` is
  ambiguous across serde versions).
- `domain/Cargo.toml` gets new optional deps + `[features]` section
  (`sqlx`, `utoipa`, both off by default). `indexer/Cargo.toml`
  enables `sqlx`; `api/Cargo.toml` enables both.
- xdr-parser now depends on `domain` (no feature flags) purely to
  consume the enum variants in `types.rs` struct fields.

### Phase 3 — parser refactor

Struct field types flipped in `crates/xdr-parser/src/types.rs`:

- `ExtractedOperation.op_type: String` → `OperationType`
- `ExtractedEvent.event_type: String` → `ContractEventType`
- `ExtractedContractDeployment.contract_type: String` → `ContractType`
- `ExtractedToken.asset_type: String` → `TokenAssetType`
- `ExtractedNftEvent.event_type: String` → `NftEventType`

Parser callsites:

- `operation.rs::extract_op_details` — return type
  `(String, Value)` → `(OperationType, Value)`; the big match returns
  `OperationType::Payment` etc. directly instead of `"PAYMENT".into()`.
- `event.rs::extract_single_event` — maps XDR
  `ContractEventType::{System,Contract,Diagnostic}` directly to
  `domain::ContractEventType` (aliased to avoid name collision).
- `state.rs::extract_contract_deployments` — `contract_type = if is_sac
{ ContractType::Token } else { ContractType::Other }` instead of
  two `.to_string()` allocations.
- `state.rs::detect_tokens` — `TokenAssetType::Sac` for SAC rows.
- `nft.rs::detect_nft_events` — filter `event.event_type ==
ContractEventType::Contract` instead of string compare.

All affected test assertions (parser: 114 unit tests) updated to
typed enum compares.

### Phase 4 — persist layer

`crates/indexer/src/handler/persist/staging.rs`:

- 6 `*Row` structs flipped (`OpRow.op_type`, `EventRow.event_type`,
  `PoolRow.asset_a_type/asset_b_type`, `NftOwnershipRow.event_type`,
  `TokenRow.asset_type`, `ContractRow.contract_type`,
  `BalanceRow.asset_type`).
- `OpTyped::from_details` signature `fn(op_type: &str, …)` →
  `fn(op_type: OperationType, …)` with `match op_type {
OperationType::Payment => …, _ => {} }` — exhaustive-optional
  (future op types fall through to the default-None branch without
  forcing a dummy arm).
- `op_participant_str_keys` same flip.
- `BalanceRow` constructor in balances loop: JSON `asset_type` string
  now `parse::<AssetType>()` — rows with unknown labels are dropped
  (can't satisfy `ck_abc_native` CHECK anyway).
- `split_pool_asset` signature changed `(String, Option<String>,
Option<String>)` → `Option<(AssetType, Option<String>,
Option<String>)>` — pools with malformed asset shape are skipped
  rather than panic'd or written with a bogus type.
- `TokenRow` identity fingerprint match: `t.asset_type.as_str()` →
  `match t.asset_type { TokenAssetType::Native => …, … }` —
  exhaustive, no fallback arm needed.
- Removed now-unused `NATIVE_ASSET_TYPE` const.
- `clone_balance_row` drops `.clone()` on `asset_type` (enum is Copy).

`crates/indexer/src/handler/persist/write.rs`:

- 6 bind sites flipped: `Vec<String>` → `Vec<EnumT>` for
  `op_type_vec` (operations), `types` (contracts), `type_vec`
  (events), `a_types`/`b_types` (pools), `types` (nft_ownership,
  balances_credit, abh_credit). sqlx encodes `Vec<EnumT>` directly
  as `SMALLINT[]` via the `sqlx::Type` derive on `#[repr(i16)]`.
- Query `UNNEST(...)` type annotations: every `$N::VARCHAR[]` that
  corresponded to an enum column → `$N::SMALLINT[]`.
- 9 inline SQL literal comparisons rewritten: `asset_type = 'native'`
  → `asset_type = 0` with inline `-- native` comment, same for
  `<> 'native'` / `IN ('classic', 'sac')` / `IN ('soroban', 'sac')`.
- `upsert_tokens_classic_like` parameter `asset_type: &str` →
  `asset_type: TokenAssetType`, binds as a positional `$1` parameter
  (no more `format!()` string injection — eliminates a minor SQL
  injection surface and keeps sqlx prepared-stmt reuse).
- `upsert_tokens_native` binds `TokenAssetType::Native`.
- `upsert_tokens_soroban` binds `TokenAssetType::Soroban`.
- `debug_assert!` added on `upsert_tokens_classic_like` to guard
  against misuse (`Classic | Sac` only).

### Phase 5 — integration tests

`crates/indexer/tests/persist_integration.rs`:

- Existing `synthetic_ledger_insert_and_replay_is_idempotent`
  fixture fields flipped to typed enums; new round-trip assertion
  fetches inserted ops as `OperationType` + `op_type_name()` label
  and asserts both forms equal expected.
- New test `enum_label_helpers_match_rust_as_str` iterates every
  variant of every enum via `check_all!` macro (43 variants × 6
  enums = 43 SQL queries), asserts `xxx_name(v as i16) = v.as_str()`.
  Skips cleanly when `DATABASE_URL` unset.

### Phase 6 — API enum serde (deferred)

As with 0151 Phase 3, API endpoint tasks (0043–0057) are still
backlog; they will consume the typed enums via the `utoipa`-derived
`ToSchema` + serde when they're implemented. The schema and helper
functions are ready.

## Design Decisions

### From Plan

1. **Edit source-of-truth migrations in place** (per 0151 precedent) —
   no new `yyyymmddHHMMSS_enum_columns.sql`. Project is pre-GA; the
   canonical DB state is whatever `0001..0008` plus the two
   timestamped ones produce.

2. **Enums live in `crates/domain/`** (not `xdr-parser/`) — domain
   already plays the role of "shared types for api + indexer".
   Parser consumes them via new `domain` dep but emits typed values
   without ever pulling sqlx/utoipa (feature-gated).

3. **Rust enum is SSoT for mapping, SQL helper is derived** — the
   integration test `enum_label_helpers_match_rust_as_str` closes
   the drift gap both ways: a new Rust variant without an SQL
   WHEN clause fails the test; an SQL WHEN clause without a Rust
   variant is caught by `VARIANTS`-driven iteration.

### Emerged

4. **`soroban_contracts.contract_type` stays NULLABLE** — ADR 0031
   §1 and task §Phase 1 both say "SMALLINT NOT NULL", but the
   two-pass upsert in `persist/write.rs` (pass-2 bare-row
   registration) inserts referenced contracts before their
   deployment meta is observed; those rows have no contract_type
   until a deploy lands. Keeping NULLABLE preserves pre-0152
   behaviour; the `ck_sc_contract_type_range` CHECK allows NULL.
   Future task (if ever) can tighten after evaluating whether to
   fabricate a synthetic `Other` default in pass-2.

5. **sqlx + utoipa as feature-gated optional deps on `domain`** —
   plan implicitly assumed they were always on. Making them
   optional keeps xdr-parser's standalone build (`cargo build -p
xdr-parser`) free of DB and HTTP toolchain, matching parser's
   pure-logic role.

6. **TryFrom + FromStr + Display everywhere, not just `as_str()`** —
   ADR 0031 only requires canonical label output. Added full
   conversion traits because: (a) sqlx decode paths need
   `TryFrom<i16>` for manual columns (future decoder work),
   (b) FromStr closes the API input-binding shape
   (`OperationType::from_str(query_param)`) without each handler
   reinventing the mapping, (c) Display for `{}`-formatting in
   tracing/error messages. Cost: ~30 lines per enum, gain:
   a clean public API surface.

7. **`split_pool_asset` returns `Option` instead of `(String,
"unknown", …)`** — previous signature returned `"unknown"`
   fallback that would silently violate the new CHECK constraint.
   Changed to `Option` and caller drops malformed pools from
   staging. Failure surfaces as a skipped pool, not a failed
   ledger.

8. **`BalanceRow` skips rows with unparseable `asset_type`** —
   instead of propagating "" / sentinel strings into the bind. Same
   rationale as 7: can't satisfy `ck_abc_native` / `ck_abh_native`
   anyway, better to skip cleanly than fail the whole ledger.

9. **`upsert_tokens_classic_like` bind parameter instead of
   `format!()`-injected literal** — existing code string-interpolated
   the asset_type into the SQL, which would have broken with a
   SMALLINT flip (need to convert enum→ordinal in SQL string).
   Changed to `.bind(asset_type)` as `$1`. Side benefit: sqlx can
   cache prepared statements per partition (classic vs sac share
   the same plan template now).

## Issues Encountered

- **FK violation on 1000-ledger bench** (discovered post-merge
  by user): at ledger 62016744 in a `--start 62016000 --end
62016999` backfill, `soroban_contracts` INSERT fails with
  `soroban_contracts_wasm_hash_fkey` because a contract references
  a `wasm_hash` uploaded in a pre-62016000 ledger (outside the
  backfill window). **Not caused by 0152** — the FK has been in
  place since ADR 0030 / task 0151; the 100-ledger bench never
  crossed this boundary because partition 62016000 happens to be
  self-contained for the contracts deployed within it. Fix goes to
  task 0153 (spawned from here).

- **Parser tests drift (114 unit tests)** — every assertion
  comparing `.op_type == "PAYMENT"` / `.event_type == "contract"` /
  `.contract_type == "other"` etc. had to flip to typed enum
  compare. Mechanical but breadth-first — used `grep -n` across
  `xdr-parser/src/*.rs` to enumerate every site, fixed 20+ sites.

- **Macro vs function in `enum_label_helpers_match_rust_as_str`** —
  first attempt used a generic `async fn<T: Copy + Into<i16>>(…)`
  helper, but #[repr(i16)] enums don't auto-impl `Into<i16>` (only
  `as i16` cast). Switched to a macro that does `*v as i16`
  directly — compiles and expresses intent (iterate every
  variant, check SQL agrees) more clearly than a 7-line generic
  fn signature.

- **serde SCREAMING_SNAKE_CASE vs. explicit renames for XDR
  AssetType** — serde's `#[serde(rename_all = "snake_case")]`
  conversion of `CreditAlphanum4` → `credit_alphanum4` is
  technically ambiguous (where does "4" go?). Used explicit
  `#[serde(rename = "credit_alphanum4")]` per variant to pin the
  label byte-for-byte to what parser state.rs has historically
  emitted in JSON (matching stellar-xdr JSON representation).

## Future Work

- **Task 0153** (spawned) — `BUG: wasm_hash FK violation on
mid-stream backfill`. Fix via stub `wasm_interface_metadata` row
  on first reference; real ABI fills in later via ON CONFLICT DO
  UPDATE COALESCE. ~30 min work. Blocks 1000+ ledger bench runs.

- **Column ordering re-layout** (ADR 0031 §4 nudge) — explicitly
  scoped out of this task but would pay ~5-10% extra heap saving
  on append-heavy tables (operations, soroban_events) by grouping
  by 8/4/2/1-byte native alignment. Candidate for a later
  maintenance task when any partitioned table is touched next.

- **Tighten `contract_type` to NOT NULL** (see Emerged #4) — would
  require changing pass-2 bare-row upsert to fabricate a default
  variant. Low priority; nullable works today.

- **API Phase 4** (per task Phase 4) — when backend module tasks
  0046, 0049, etc. resume, each handler decodes SMALLINT →
  Rust enum → serde emits canonical string. No new code needed
  in `domain/`; just use the existing `utoipa::ToSchema` derive
  for OpenAPI and let serde render the JSON. Tracked implicitly
  in those backlog tasks — no separate spawn needed.
