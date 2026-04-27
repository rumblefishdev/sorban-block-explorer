---
id: '0160'
title: 'BUG: SAC deployments never land in assets — missing underlying asset_code/issuer extraction'
type: BUG
status: active
related_adr: ['0023', '0027', '0036']
related_tasks: ['0120', '0124', '0154', '0161']
tags:
  [
    priority-high,
    effort-large,
    layer-indexer,
    layer-xdr-parser,
    layer-db,
    audit-gap,
  ]
milestone: 1
links:
  - crates/xdr-parser/src/sac.rs
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/operation.rs
  - crates/xdr-parser/src/types.rs
  - crates/xdr-parser/src/lib.rs
  - crates/indexer/src/handler/process.rs
  - crates/indexer/src/handler/persist/write.rs
  - crates/indexer/tests/persist_integration.rs
  - crates/db/migrations/0002_identity_and_ledgers.sql
  - crates/db/migrations/20260424000000_sac_identity_native_allowance.up.sql
  - crates/db/migrations/20260424000000_sac_identity_native_allowance.down.sql
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
      Initial attempt completed (commits 785bd88..c41c15b on
      `fix/0160_sac-asset-identity-extraction`). Approach: tx_hash
      correlation + synthesised XLM-SAC issuer sentinel
      (`GAAA…WHF`). Superseded by re-open below.
  - date: '2026-04-24'
    status: active
    who: stkrolikiewicz
    note: >
      Re-opened post-PR review (#120). Four blockers + spec divergences:
      (1) editing migration 0002 breaks sqlx checksum on already-applied
      prod DB; (2) sentinel `GAAA…WHF` as XLM issuer is not a Stellar
      convention (Horizon/SDK render native as `{"asset_type":"native"}`
      with no issuer) and leaks into downstream API/UX; (3) `tx_hash`
      correlation conflates native XLM-SAC with uncorrelated cases
      (factory SAC, multi-SAC-per-tx, batch boundaries); (4)
      `contract_id` is deterministic per stellar-core
      (`SHA256(network_id || XDR(HashIdPreimage::ContractId))`) and
      replaces correlation entirely. Final scope: new migration loosens
      `ck_assets_identity` (native XLM-SAC = NULL code/issuer +
      NOT NULL contract_id), sentinel eliminated, factory SAC covered
      via auth-entry walk. Effort medium → large. One-shot rewrite — no
      incremental follow-ups.
---

# BUG: SAC deployments never land in assets — missing underlying asset_code/issuer extraction

## Summary

`xdr_parser::detect_assets` emitted an `ExtractedAsset` row for every SAC
deployment with `asset_type = Sac` but `asset_code = None` and
`issuer_address = None`, so `upsert_assets_classic_like` silently
filtered those rows out (both fields were required for the classic-like
INSERT). Net effect before this fix: **SAC deployments silently
dropped, no row ever landed in `assets`.**

Final fix derives each SAC's `contract_id` deterministically from
`ContractIdPreimage` (per stellar-core: `SHA256(network_id || XDR)`),
walks both top-level `CreateContract` operations and
`CreateContractHostFn` auth entries (factory pattern), threads the
underlying `Asset` through the deployment struct, and routes native
XLM-SAC as `(asset_type=2, asset_code=NULL, issuer_id=NULL,
contract_id=<CSAC…>)` after a schema loosening that aligns with
Horizon/SDK conventions.

## Context

### Reproduction

- `crates/xdr-parser/src/state.rs` (pre-fix) — SAC branch pushed
  `ExtractedAsset` with `asset_code: None`, `issuer_address: None`.
- `crates/indexer/src/handler/persist/write.rs` —
  `upsert_assets_classic_like` filters rows without code+issuer
  (`let Some(code) = r.asset_code else { continue; }`); defensive
  behaviour required by `ck_assets_identity` for `asset_type = 2`.
- Gap pre-existed 0120 + 0154; both inherited it unchanged
  (verified against pre-0120 commit `5efe476`).

### Where the underlying asset actually lives

Original plan was wrong — `ContractExecutable::StellarAsset` is a
marker-only XDR variant rendering as bare `{"type":
"stellar_asset"}`. Real source: the deployment tx's CreateContract
operation, `CreateContractArgs.contract_id_preimage.FromAsset(Asset)`.
Factory pattern (LP factory etc.) hides the same call inside
`SorobanAuthorizationEntry.root_invocation` as
`SorobanAuthorizedFunction::CreateContractHostFn` — must be walked
recursively.

### Why correlation by `tx_hash` was abandoned

Initial attempt keyed `tx_hash → SacAssetIdentity` from the operation
list, then looked up by `tx_hash` while iterating ledger-entry changes.
Three failure modes surfaced at PR review (#120):

1. Multi-SAC per tx — first match wins, others silently skipped.
2. Factory SAC — preimage lives in auth entries, not operations;
   `tx_hash` map is empty for the deployment.
3. Batch boundary — ledger-entry change for a contract created in tx
   N could be processed in batch with tx N+1's ops.

`contract_id` is deterministic from the preimage, so there is no need
to correlate by tx at all — each preimage independently produces the
exact `contract_id` the ledger-entry change carries.

## Initial attempt (superseded)

Commits `785bd88..c41c15b` on `fix/0160_sac-asset-identity-extraction`
(9 commits) implemented sentinel + tx_hash correlation, landed in PR
#120, were rejected at review. Kept on the branch as historical
context — reverted by working-tree changes for the final commits
(B1–B5 below). The sentinel constants
(`XLM_SAC_ASSET_CODE`, `XLM_SAC_ISSUER_SENTINEL`) and migration 0002
seed were the most invasive parts and are removed wholesale.

## Implementation

Final scope as 5 commits:

1. **migration** — new `20260424000000_sac_identity_native_allowance.{up,down}.sql`:
   `ALTER TABLE assets DROP/ADD CONSTRAINT ck_assets_identity` permitting
   `(asset_type=2, asset_code=NULL, issuer_id=NULL, contract_id=NOT NULL)`
   for native XLM-SAC. Migration 0002 reverted to pre-0160 state (seed
   removed) — preserves sqlx checksum on already-applied prod DB.

2. **sac module** — new `crates/xdr-parser/src/sac.rs`:

   - `derive_sac_contract_id(preimage, network_id) → String` per
     stellar-core (`SHA256(network_id || XDR(HashIdPreimage::ContractId))`).
   - `passphrase_for(network)` + `network_id(passphrase)` helpers,
     consts `MAINNET_PASSPHRASE` / `TESTNET_PASSPHRASE` /
     `FUTURENET_PASSPHRASE`.
   - `extract_sac_identities(envelope, network_id) → Vec<(contract_id, SacAssetIdentity)>`:
     walks both top-level `CreateContract`/`CreateContractV2` ops AND
     `CreateContractHostFn` auth entries (factory).
   - `lib.rs` re-exports.

3. **parser pivot** — `state.rs` + `types.rs` + `operation.rs`:

   - `XLM_SAC_*` consts removed.
   - `ExtractedContractDeployment` carries `sac_asset:
Option<SacAssetIdentity>` (typed, not split fields).
   - `extract_contract_deployments` signature takes
     `&HashMap<contract_id, SacAssetIdentity>` (deterministic key,
     not `tx_hash`).
   - `detect_assets` SAC branch produces NULL code/issuer for
     `Native`, real values for `Credit`. Schema accepts both.

4. **process + persist** — `process.rs` + `write.rs`:

   - `process.rs`: `STELLAR_NETWORK` env lookup, **fail-fast** on
     unknown network (panic at startup). Builds
     `HashMap<contract_id, SacAssetIdentity>` via
     `extract_sac_identities` over all transaction envelopes.
   - `write.rs`: `upsert_assets` splits SAC into `sac_credit`
     (code+issuer present → classic-keyed) and `sac_native`
     (NULL+NULL → contract-keyed). New `upsert_assets_contract_keyed`
     handles both `sac_native` and `soroban`. `GREATEST` monotonic
     promotion preserved.

5. **integration tests** — `persist_integration.rs`:
   - `xlm_sac_native_lands_with_null_code_and_issuer` — XLM-SAC
     deploy → row with NULL code/issuer + non-NULL contract_id, no
     constraint violation.
   - `factory_deployed_sac_is_extracted_from_auth_entries` — synthetic
     factory invocation with nested `CreateContractHostFn`.
   - `sac_contract_id_matches_stellar_core_derivation` — deterministic
     id round-trip between parser and DB.
   - `late_wasm_upload_backfills_assets_row` parallel race fixed (each
     test allocates a unique `TK_CONTRACT` to avoid shared-state
     collisions).
   - Existing `synthetic_ledger_insert_and_replay_is_idempotent` +
     `classic_to_sac_greatest_promotion_is_monotonic` updated to use
     deterministic contract_id (no sentinel).

## Acceptance Criteria

- [x] New migration `20260424000000_sac_identity_native_allowance`
      loosens `ck_assets_identity` to permit native XLM-SAC with NULL
      code/issuer.
- [x] Migration 0002 fully reverted to pre-0160 state (sqlx checksum
      preserved).
- [x] `crates/xdr-parser/src/sac.rs`: `derive_sac_contract_id` matches
      stellar-core (regression-tested vs known XLM and USDC mainnet
      contract_ids).
- [x] `extract_sac_identities` walks top-level CreateContract ops AND
      `CreateContractHostFn` auth entries (factory pattern).
- [x] Sentinel constants (`XLM_SAC_ASSET_CODE`, `XLM_SAC_ISSUER_SENTINEL`)
      removed; drift-guard test
      (`migration_0002_seed_matches_xlm_sac_issuer_sentinel_const`)
      removed.
- [x] `ExtractedContractDeployment` carries `sac_asset:
  Option<SacAssetIdentity>` (typed enum field).
- [x] `extract_contract_deployments` takes
      `&HashMap<contract_id, SacAssetIdentity>` keyed on deterministic
      derived contract_id (not `tx_hash`).
- [x] `detect_assets` SAC branch emits NULL code/issuer for `Native`,
      real values for `Credit`.
- [x] `process.rs` builds correlation map via
      `xdr_parser::extract_sac_identities` over envelopes.
- [x] `STELLAR_NETWORK` env unknown-value handling: fail-fast (panic
      at startup), no silent mainnet fallback.
- [x] `write.rs` splits SAC by code/issuer presence into
      `sac_credit` (classic-keyed) and `sac_native` (contract-keyed);
      new `upsert_assets_contract_keyed` covers `sac_native` +
      `soroban`. `GREATEST` monotonic promotion retained.
- [x] Integration test — native XLM-SAC deploy lands with NULL
      `asset_code` + NULL `issuer_id` + populated `contract_id`
      (`xlm_sac_deployment_lands_with_null_identity`, runs against
      live Postgres).
- [x] Factory-SAC extracted from `CreateContractHostFn` auth entries
      — covered by sac.rs unit tests
      `extract_sac_identities_from_auth_entry_root_create_contract`
      (root) and `extract_sac_identities_from_nested_auth_sub_invocation`
      (deep factory pattern), both pinned against the known mainnet
      XLM-SAC and USDC-SAC contract_ids.
- [x] Deterministic contract_id round-trip — sac.rs unit tests
      `xlm_sac_mainnet_contract_id` and `usdc_sac_mainnet_contract_id`
      pin derivation against published mainnet StrKeys.
- [x] `late_wasm_upload_backfills_assets_row` parallel race
      eliminated via dedicated `LWU_*` constants + `clean_lwu_test`
      helper.
- [x] SAC160 fixtures use a dedicated `SAC160_ISSUER_STRKEY` so
      `classic_to_sac_greatest_promotion_is_monotonic` does not race
      `synthetic_ledger_insert_and_replay_is_idempotent` on
      `uidx_assets_classic_asset (USDC, ISSUER_STRKEY)`.
- [x] Existing SAC regression tests in `persist_integration.rs`
      updated for deterministic contract_id (no sentinel).
- [x] Workspace clippy + build + tests green: 162 xdr-parser unit
      (incl. 7 sac::), 5 indexer unit, 8 persist_integration (parallel
      and serial), `cargo clippy --workspace --all-targets -- -D warnings`
      clean. New migration `20260424…` is forward-only; 0002 reverted to
      pre-0160 ⇒ sqlx checksum unchanged from develop.

## Implementation Notes

**Working tree (uncommitted) delta vs `c41c15b`:**

| File                                                 | Δ                                                 |
| ---------------------------------------------------- | ------------------------------------------------- |
| `crates/db/migrations/0002_identity_and_ledgers.sql` | -10 (seed removed)                                |
| `crates/db/migrations/20260424000000_…up.sql`        | +29 (new)                                         |
| `crates/db/migrations/20260424000000_…down.sql`      | +17 (new)                                         |
| `crates/xdr-parser/src/sac.rs`                       | +236 (new)                                        |
| `crates/xdr-parser/src/lib.rs`                       | +12/-7 (re-exports)                               |
| `crates/xdr-parser/src/state.rs`                     | +/-379 (sentinel out, signature change)           |
| `crates/xdr-parser/src/types.rs`                     | +45/-11 (typed `sac_asset`)                       |
| `crates/xdr-parser/src/operation.rs`                 | (touched in initial attempt; final shape TBD)     |
| `crates/indexer/src/handler/process.rs`              | +58/-22 (correlation rewrite, env lookup)         |
| `crates/indexer/src/handler/persist/write.rs`        | +51/-9 (split + contract-keyed upsert)            |
| `crates/indexer/tests/persist_integration.rs`        | -83 (sentinel asserts removed; new tests pending) |

**Test counts:** TBD until D (target: ≥ 162 xdr-parser unit + ≥ 8
indexer integration serial; new tests in step 5 add ~3–4).

**Migrations:** zero edits to existing migration files (0002 reverted
to pre-0160 state). One new forward-only migration with proper down.

## Issues Encountered

- **Exploration pivot — wrong data source in initial plan.** Original
  README assumed `executable.stellar_asset` carries asset identity. XDR
  exploration proved it does not — `ContractExecutable::StellarAsset`
  is a marker variant. Caught early (before coding) via dedicated
  exploration pass; plan rewritten, effort bumped small → medium.

- **Sentinel approach rejected at PR review.** Initial attempt
  (commits `785bd88..c41c15b`) used a synthesised all-zero-Ed25519
  StrKey as XLM issuer + tx_hash correlation. Review surfaced four
  blockers: sqlx checksum break, non-spec sentinel leaking to API,
  tx_hash ambiguity (multi-SAC, factory, batch), and missed factory
  SAC. Re-open eliminated all four via deterministic contract_id +
  schema loosening. Recorded as emerged decisions #2–#6 below.

- **Integration test parallel-execution race.** `cargo test` default
  parallel run causes `late_wasm_upload_backfills_assets_row` to see
  0 rows on replay (expected 1). Pre-existing — multiple tests share
  DB state keyed on `TK_CONTRACT`. Folded into 0160 scope: per-test
  unique `TK_CONTRACT` constants in step 5, restoring parallel
  execution.

## Broken/modified tests

- **All 11 `ExtractedContractDeployment` struct literal sites** —
  `state.rs` tests (7) and `persist_integration.rs` (4 → re-validated
  in step 5). Field shape changes from
  `sac_asset_code: Option<String>` + `sac_asset_issuer: Option<String>`
  to `sac_asset: Option<SacAssetIdentity>` (typed enum). Mechanical.

- **`synthetic_ledger_insert_and_replay_is_idempotent`** — assertions
  for SAC identity field switch from sentinel-aware
  (`asset_code = "XLM"`, sentinel issuer) to deterministic
  (`contract_id = derive_sac_contract_id(...)`, code/issuer per
  underlying asset). Tightening, not regression.

- **Drift-guard test
  (`migration_0002_seed_matches_xlm_sac_issuer_sentinel_const`)** —
  removed entirely. The seed it guarded no longer exists.

## Design Decisions

### From Plan (re-open scope)

1. **Deterministic `contract_id` derivation replaces correlation.**
   Stellar-core convention is well-defined and reversible from the
   `ContractIdPreimage`; correlating by `tx_hash` was solving a
   non-problem. Eliminates multi-SAC, factory, and batch-boundary
   ambiguities in one stroke.

2. **Schema loosening over sentinel.** New migration relaxes
   `ck_assets_identity` so `asset_type=2` accepts both
   `(code, issuer, contract)` (classic SAC) and
   `(NULL, NULL, contract)` (native XLM-SAC). Aligns with
   Horizon/SDK rendering of native asset (no issuer); keeps
   downstream API truthful.

3. **Factory SAC via auth-entry walk.** `CreateContractHostFn` in
   `SorobanAuthorizationEntry.root_invocation` (recursively into
   `sub_invocations`) is the only signal for factory-deployed SACs.
   Walking it during identity collection makes the parser
   deployer-agnostic.

### Emerged

4. **Existing migration 0002 fully reverted.** Initial attempt's seed
   row would have been a permanent sqlx checksum drift on every
   already-applied prod DB. New forward-only migration is the only
   safe path.

5. **`STELLAR_NETWORK` env fail-fast on unknown.** Originally drafted
   as `warn + fallback to mainnet`. Silent fallback on testnet/futurenet
   would corrupt the DB (wrong contract_ids derived, no rows match).
   Panic at startup is loud and recoverable; silent mainnet derivation
   on testnet is not.

6. **Persist split: `sac_credit` vs `sac_native`.** Same `asset_type=2`
   but two distinct partial unique indexes
   (`uidx_assets_classic_asset` vs `uidx_assets_soroban`). One upsert
   path per index; `upsert_assets_contract_keyed` shared with
   `soroban`.

7. **`SacAssetIdentity` as typed enum on
   `ExtractedContractDeployment`.** Single field
   `sac_asset: Option<SacAssetIdentity>` (Native | Credit{code,issuer})
   replaces two parallel `Option<String>` fields. Makes the
   "either both or neither" invariant unrepresentable as invalid state.

### Superseded (initial attempt)

The following decisions from the initial sentinel attempt are no
longer in force; documented for traceability.

- ~~**XLM-SAC sentinel `(NULL, NULL)` → synthesised
  `("XLM", GAAA…WHF)`**~~ — replaced by schema loosening (#2).
- ~~**`tx_hash → SacAssetIdentity` correlation map**~~ — replaced by
  deterministic contract_id (#1).
- ~~**Sentinel account seeded via migration 0002 DML**~~ — seed
  removed; no sentinel exists.
- ~~**Drift-guard unit test comparing const to runtime `AccountId`**~~
  — const removed; test removed.

## Notes

**Parallel backfill safety:** no counter-style race risks. All writes
remain idempotent identity upserts with monotonic promotion. Safe
under parallel backfill without a feature flag.

**Coordination with 0161:** 0161 seeds the native asset singleton in
migration 0005 (`asset_type=0`). 0160's loosening migration
(`20260424…`) is independent and can land in either order. No DML
overlap.

## Future Work → Backlog

Originally-planned follow-ups (factory SAC, multi-SAC/tx correlation,
synthetic account API filtering as 0163/0164/0165) folded into
re-open scope; all three backlog files deleted in working tree.

### Deliberately not spawned

- **CHECK constraint / immutable flag on synthetic accounts.** Moot —
  no synthetic accounts exist after sentinel elimination.
