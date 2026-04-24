---
id: '0160'
title: 'BUG: SAC deployments never land in assets — missing underlying asset_code/issuer extraction'
type: BUG
status: completed
related_adr: ['0023', '0027', '0036']
related_tasks: ['0120', '0124', '0154', '0161', '0163', '0164']
tags:
  [
    priority-high,
    effort-medium,
    layer-indexer,
    layer-xdr-parser,
    layer-db,
    audit-gap,
  ]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/operation.rs
  - crates/xdr-parser/src/types.rs
  - crates/xdr-parser/src/lib.rs
  - crates/indexer/src/handler/process.rs
  - crates/indexer/src/handler/persist/write.rs
  - crates/indexer/tests/persist_integration.rs
  - crates/db/migrations/0002_identity_and_ledgers.sql
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from post-0154 audit. Empirical: after reindexing 1000
      ledgers on develop, `assets` table is empty. Root cause is
      pre-existing (inherited through 0120 + 0154 rename, both
      mechanical). Not a regression.
  - date: '2026-04-24'
    status: active
    who: stkrolikiewicz
    note: >
      Activated. Highest-impact of the three audit-gap tasks (0160/0161/0162)
      — SAC/classic is the dominant asset population on mainnet, so this
      directly unblocks `assets` table completeness.
  - date: '2026-04-24'
    status: active
    who: stkrolikiewicz
    note: >
      Exploration pivot. Original plan assumed `executable.stellar_asset`
      subtree carries asset_code/issuer — it does not. XDR
      `ContractExecutable::StellarAsset` is a marker variant with no
      embedded asset data. Real source: deployment tx's CreateContract
      operation args, `ContractIdPreimage::FromAsset(Asset)`. Plan
      rewritten. Effort bumped small → medium.
  - date: '2026-04-24'
    status: completed
    who: stkrolikiewicz
    note: >
      Done. 6 commits on fix/0160_sac-asset-identity-extraction. 12 new
      xdr-parser unit tests (162 total pass), 2 new integration tests
      + 1 enhanced regression assertion (8/8 pass against live Postgres
      under --test-threads=1). Migration 0002 seeds the all-zero-Ed25519
      sentinel account; `upsert_assets_classic_like` DO UPDATE now uses
      `asset_type = GREATEST(...)` for monotonic parallel-safe
      promotion. Spawned follow-ups 0163 (factory-SAC) and 0164
      (multi-SAC-per-tx) per the known coverage gaps.
      cargo clippy --workspace -- -D warnings clean.
---

# BUG: SAC deployments never land in assets — missing underlying asset_code/issuer extraction

## Summary

`xdr_parser::detect_assets` emitted an `ExtractedAsset` row for every SAC
deployment with `asset_type = Sac` but `asset_code = None` and
`issuer_address = None`, so `upsert_assets_classic_like` silently
filtered those rows out (both fields were required for the classic-like
INSERT). Net effect before this fix: **SAC deployments silently
dropped, no row ever landed in `assets`.** Fix reads the classic asset
out of the creating transaction's `CreateContract.contract_id_preimage`,
threads it through the deployment struct, and applies a synthesised
sentinel for XLM-SAC (native wrap) — no schema change.

## Context

### Reproduction

- `crates/xdr-parser/src/state.rs:513-522` (before fix) — SAC branch
  pushed `ExtractedAsset` with `asset_code: None`, `issuer_address:
None`.
- `crates/indexer/src/handler/persist/write.rs:1095-1101` —
  `upsert_assets_classic_like` filters rows without code+issuer
  via `let Some(code) = r.asset_code else { continue; };`. Defensive
  behaviour (without it the INSERT would violate
  `ck_assets_identity` for `asset_type = 2`) — the real fix was
  upstream.
- Gap pre-existed 0120 + 0154; both commits inherited it unchanged
  (verified against pre-0120 commit `5efe476`).

### Where the underlying asset actually lives

**Initial plan was wrong.** Original README assumed the
`executable.stellar_asset` ledger-entry subtree carries asset identity.
It does not — `ContractExecutable::StellarAsset` is a marker-only XDR
variant rendering as bare `{"type": "stellar_asset"}` (confirmed in
`scval.rs:75-84`). Real source: the **deployment tx's CreateContract
operation**, specifically
`CreateContractArgs.contract_id_preimage.FromAsset(Asset)`. Parser saw
it but discarded it in `operation.rs:328-346`.

Timing: `process.rs:130` loop over `all_ledger_entry_changes` runs
after `all_operations` is populated. `tx_hash` already in the tuple
(was `_tx_hash`). Correlation cheap + co-located.

## Implementation

Implemented as 6 commits on `fix/0160_sac-asset-identity-extraction`:

1. **`785bd88` docs(lore-0160)** — rewrite plan post-exploration pivot.
2. **`9bfc222` feat(lore-0160)** — `operation.rs`: new helpers
   `format_asset_structured` + `format_contract_id_preimage`; extend
   CreateContract / CreateContractV2 arms to include
   `contractIdPreimage` in `ExtractedOperation.details`. 5 unit tests.
3. **`1ed1a35` feat(lore-0160)** — `state.rs`: new enum
   `SacAssetIdentity { Native, Credit { code, issuer } }` + pure fn
   `extract_sac_asset_from_create_contract`. Defensive parsing
   returns `None` for non-InvokeHostFunction ops, non-CreateContract
   host functions, FromAddress preimages, malformed JSON. 7 unit
   tests.
4. **`19b1869` feat(lore-0160)** — parser path end-to-end:
   - `ExtractedContractDeployment` gains typed `sac_asset_code`
     - `sac_asset_issuer`.
   - `extract_contract_deployments` signature extended with
     `sac_asset_identity: Option<&SacAssetIdentity>`.
   - `detect_assets` SAC branch reads the deployment fields and
     applies the XLM-SAC sentinel for None/None.
   - `process.rs` builds `HashMap<tx_hash, SacAssetIdentity>` from
     `all_operations` before the deployment loop, threads per-tx.
   - New public consts `XLM_SAC_ASSET_CODE = "XLM"` +
     `XLM_SAC_ISSUER_SENTINEL =
"GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF"`.
   - Fixture updates across 11 `ExtractedContractDeployment`
     literals (7 in state.rs tests, 5 in persist_integration.rs).
   - 3 additional unit tests in state.rs (sentinel round-trip,
     sentinel StrKey validation).
5. **`8cb44f0` feat(lore-0160)** — persist side:
   - `upsert_assets_classic_like` DO UPDATE gains
     `asset_type = GREATEST(EXCLUDED.asset_type, assets.asset_type)`
     for monotonic parallel-safe promotion.
   - Migration `0002_identity_and_ledgers.sql` seeds the sentinel
     account row (single DML `INSERT`). Zero DDL.
6. **`097af85` test(lore-0160)** — integration tests:
   - Enhanced `synthetic_ledger_insert_and_replay_is_idempotent`
     with SAC identity field assertions (asset_code, issuer_id,
     contract_id all non-NULL).
   - New `xlm_sac_deployment_lands_with_sentinel_identity` —
     XLM-SAC round-trip, asserts sentinel lands and FK resolves.
   - New `classic_to_sac_greatest_promotion_is_monotonic` —
     order-swap: SAC(type=2) committed first, ClassicCredit(type=1)
     second; final row is type=2 with contract_id preserved, no
     `ck_assets_identity` violation.
   - `xdr-parser` crate re-exports `XLM_SAC_ASSET_CODE` +
     `XLM_SAC_ISSUER_SENTINEL` for test consumers.

## Acceptance Criteria

- [x] `operation.rs` extracts `contract_id_preimage` for both
      `CreateContract` and `CreateContractV2`; helper
      `format_contract_id_preimage` handles `FromAddress` + `FromAsset`.
- [x] `format_asset_structured` helper covers `Native`,
      `CreditAlphanum4`, `CreditAlphanum12` variants, output shape
      matches trustline convention (`type`, `asset_code`, `issuer`).
      Helper renamed from `format_asset` (collision with pre-existing
      compact-string helper that is consumed elsewhere).
- [x] `process.rs` builds tx_hash → SAC-asset map before deployment
      extraction loop, threads it into `extract_contract_deployments`.
- [x] `ExtractedContractDeployment` gains `sac_asset_code`,
      `sac_asset_issuer` typed fields.
- [x] `detect_assets` SAC branch populates `ExtractedAsset.asset_code`
      and `.issuer_address` from deployment fields.
- [x] XLM-SAC (native preimage) emits sentinel
      (`"XLM"`, all-zero StrKey) — sentinel account seeded via
      migration 0002 (not 0005 as initially planned; 0002 is where
      `accounts` is declared).
- [x] Issuer G-addresses added to `staging.account_keys` so
      `upsert_accounts` resolves `issuer_id` before
      `upsert_assets_classic_like` runs. **Pre-existing in
      `staging.rs:365-368`** — no new code needed.
- [x] `upsert_assets_classic_like` DO UPDATE uses
      `asset_type = GREATEST(EXCLUDED.asset_type, assets.asset_type)`
      for monotonic parallel-safe promotion.
- [x] Unit test coverage: 12 new xdr-parser tests (operation +
      state + sentinel roundtrip) across two commits. 162/162 pass.
- [x] Integration test — SAC credit deploy → `assets` row with all
      three identity fields non-NULL (enhanced existing
      `synthetic_ledger_insert_and_replay_is_idempotent`).
- [x] Integration test — XLM-SAC deploy → row with sentinel identity
      (`xlm_sac_deployment_lands_with_sentinel_identity`).
- [x] Integration test — classic↔SAC order-swap does not violate
      `ck_assets_identity` (`classic_to_sac_greatest_promotion_is_monotonic`).
- [x] Existing SAC regression tests in `persist_integration.rs`
      updated to assert populated identity fields.
- [x] `nx run rust:build` / `rust:test` / `rust:lint` green. 8/8
      persist_integration tests pass serial; 162/162 xdr-parser pass.

## Implementation Notes

**Commits:** 6 on branch `fix/0160_sac-asset-identity-extraction`
(`785bd88`, `9bfc222`, `1ed1a35`, `19b1869`, `8cb44f0`, `097af85`).

**Files touched:**

| File                                                 | Change                                                                                                   |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `crates/xdr-parser/src/operation.rs`                 | +205 (2 helpers, CreateContract/V2 preimage extraction, 5 tests)                                         |
| `crates/xdr-parser/src/state.rs`                     | +185 (sentinel consts, SacAssetIdentity, extract fn, signature change, detect_assets sentinel, 10 tests) |
| `crates/xdr-parser/src/types.rs`                     | +11 (2 new struct fields)                                                                                |
| `crates/xdr-parser/src/lib.rs`                       | +5 (public re-exports)                                                                                   |
| `crates/indexer/src/handler/process.rs`              | +22 (correlation map build + call site)                                                                  |
| `crates/indexer/src/handler/persist/write.rs`        | +12 (GREATEST + comment)                                                                                 |
| `crates/indexer/tests/persist_integration.rs`        | +421 (11 struct-literal updates, 2 new tests, 1 enhanced, new fixture constants + cleanup helper)        |
| `crates/db/migrations/0002_identity_and_ledgers.sql` | +9 (DML seed for sentinel account)                                                                       |

**Test counts:** 162/162 xdr-parser unit (+12 new). 8/8 indexer
integration serial (+2 new, +1 enhanced). `cargo clippy --workspace
-- -D warnings` clean.

**Zero DDL.** Only DML: one `INSERT INTO accounts` in migration 0002.
All touched columns already existed in 0005.

## Issues Encountered

- **Exploration pivot — wrong data source in initial plan.** Original
  README assumed `executable.stellar_asset` carries asset identity.
  XDR exploration proved it does not — `ContractExecutable::StellarAsset`
  is a marker variant. Caught early (before coding) via dedicated
  exploration pass; plan rewritten, effort bumped small → medium.
  Not a regression — pure planning error. Captured as emerged
  decision #2 below.

- **Sentinel StrKey typo (55 vs 56 chars).** First hard-coded constant
  had one too few `A` chars
  (`GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF` — 55
  chars). The runtime-compare test
  `xlm_sac_issuer_sentinel_const_matches_all_zero_ed25519_strkey`
  caught it immediately by comparing the const to
  `AccountId(Uint256([0; 32])).to_string()`. Fixed to 56 chars
  (`GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF`).
  The defensive regression test exists specifically to catch this
  drift vector between the Rust const and the migration-seeded
  string.

- **Integration test parallel-execution race.** `cargo test` default
  parallel run caused `late_wasm_upload_backfills_assets_row` to
  see 0 rows on replay (expected 1). Pre-existing issue — tests
  share DB state keyed on `TK_CONTRACT`, multiple tests touching the
  same contract race. Passes cleanly under `--test-threads=1`. Not
  caused by 0160 changes; noted for potential follow-up if the
  test suite grows.

## Broken/modified tests

- **`synthetic_ledger_insert_and_replay_is_idempotent`** — added
  identity-field assertions after the existing count assertions.
  Before this task the test passed even with silently-dropped SAC
  rows (counts-only check masked the bug). Now queries
  `(asset_code, issuer_id, contract_id)` for the SAC row and
  asserts all three non-NULL + `asset_code = "USDC"`. Intentional
  tightening, not a regression.

- **11 `ExtractedContractDeployment` struct literal sites** — across
  `state.rs` tests (7) and `persist_integration.rs` (5). Added
  `sac_asset_code` / `sac_asset_issuer` fields. Non-SAC literals got
  `None`/`None`; SAC literals got `Some("USDC")` + real issuer StrKey
  so tests exercise populated identity. Structural change, not
  semantic.

## Design Decisions

### From Plan

1. **Populate SAC identity in `ExtractedContractDeployment` (typed
   fields, not `metadata: Value`)** — identity data deserves typed
   fields; free-form `metadata` stays for future SEP-1 / on-chain
   extensions (0124, 0156).

### Emerged

2. **Data source pivot — operation, not ContractInstance.** Original
   plan assumed `executable.stellar_asset` subtree carries asset
   identity. XDR exploration proved it does not
   (`ContractExecutable::StellarAsset` is a marker variant). Pivoted
   to reading `CreateContractArgs.contract_id_preimage.FromAsset` from
   the deployment transaction's operation payload. Captured in
   `785bd88` README rewrite.

3. **XLM-SAC → option (c) synthesised sentinel.** Rejected option
   (a) (schema CHECK loosening — invasive) and (b) (route as
   `Soroban` — semantic mismatch). Chose sentinel
   (`"XLM"`, all-zero StrKey) to preserve `asset_type = 2` semantics
   without schema change.

4. **Sentinel account seeded via migration 0002, not 0005.**
   Original plan said "0005 or combined DML with 0161". Chose 0002
   because `accounts` table is declared there — the seed row must
   live adjacent to its owning table, not orphaned in a later
   migration. 0161's native asset singleton will still land in 0005
   where `assets` is declared.

5. **New helper `format_asset_structured`, not reshaping existing
   `format_asset`.** Pre-existing `format_asset` (`operation.rs:352`)
   returns a compact `"CODE:ISSUER"` string used by ~15 call sites
   (payment / path-payment / manage-offer / trustline-op / etc.).
   Reshape would break all of them. New helper returns structured
   `{type, asset_code, issuer}` for consumers that need separated
   fields — kept the old API stable.

6. **Tx-correlation by `tx_hash` assumes 1:1 CreateContract per tx.**
   Multiple SAC deploys in a single tx theoretically possible but
   rare in practice. Spawned follow-up **0164** to replace with
   preimage-hash ↔ contract_id correlation if metrics show >1 on
   mainnet.

7. **Factory-deployed SACs (via inner invocations) out of scope.**
   `invocation.rs:248` sees `CreateContractHostFn` in auth entries
   but this task only correlates top-level operation CreateContract.
   Spawned follow-up **0163** for the auth-entry path.

8. **`asset_type` monotonic promotion via `GREATEST`.** Not part of
   the original SAC-identity scope, but surfaces immediately once SAC
   rows start actually INSERTing — a future classic-extraction path
   committing out of order could downgrade `asset_type` from Sac(2)
   to ClassicCredit(1) and violate `ck_assets_identity`. Order-
   independent, parallel-safe, zero-cost defensive now + correct
   when classic extraction lands.

9. **Step 5 (issuer → `account_keys` staging) turned out to be
   no-op.** `staging.rs:365-368` already iterates `assets` and
   inserts `issuer_address` into `account_keys_set`. Because
   `detect_assets` now populates `issuer_address` (real or sentinel),
   the pre-existing code automatically routes the G-address to
   `upsert_accounts`. Saved a commit; verified via integration test
   that FK resolution succeeds end-to-end.

10. **Sentinel StrKey regression guard via runtime compare.** Added
    `xlm_sac_issuer_sentinel_const_matches_all_zero_ed25519_strkey`
    unit test. Catches drift between the hard-coded Rust const, the
    migration-seeded string, and the stellar-xdr encoding. Saved a
    production FK failure when the first-draft const had one fewer
    `A` than required.

## Notes

**Parallel backfill safety:** this task does NOT introduce
counter-style race risks (no +1/-1 inline updates). All writes are
idempotent identity upserts with monotonic promotion. Safe to run
under parallel backfill without a feature flag.

**Coordination with 0161:** 0160 seeds the sentinel _account_ in
migration 0002; 0161 seeds the native _asset_ in migration 0005.
Independent DML edits in different migrations; either PR can land
first.

## Future Work → Backlog

Spawned as separate tasks per `/lore-framework-tasks` "never leave
future work as prose only" rule:

- **0163** — Factory-deployed SAC detection (via
  `CreateContractHostFn` auth-entry path).
- **0164** — Multi-SAC-per-tx correlation (preimage-hash ↔
  contract_id when >1 SAC deploys per tx).
