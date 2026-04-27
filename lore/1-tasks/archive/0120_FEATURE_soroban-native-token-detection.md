---
id: '0120'
title: 'Indexer: detect Soroban-native tokens (non-SAC) and persist to assets table'
type: FEATURE
status: completed
related_adr: ['0027', '0030', '0031']
related_tasks: ['0027', '0049', '0104', '0118', '0149']
tags: [priority-medium, effort-small, layer-indexer, audit-F8]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/classification.rs
  - crates/xdr-parser/src/types.rs
  - crates/indexer/src/handler/persist/staging.rs
  - crates/indexer/src/handler/persist/write.rs
  - crates/db/migrations/0005_tokens_nfts.sql
  - lore/2-adrs/0031_enum-columns-smallint-with-rust-enum.md
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F8 (MEDIUM severity).'
  - date: '2026-04-15'
    status: active
    who: FilipDz
    note: 'Activated for implementation.'
  - date: '2026-04-23'
    status: active
    who: stkrolikiewicz
    note: >
      Ownership handover from FilipDz.
  - date: '2026-04-23'
    status: active
    who: stkrolikiewicz
    note: >
      SCOPE REWRITE. Original spec (classify + insert `assets` row)
      was written pre-0118-Phase-2 and pre-ADR-0031, referencing
      VARCHAR string enums and assuming classification was missing.
      Current repo state (post-PR #110, commit `91eec4f`):

      ALREADY DONE by upstream work:
      - `ContractType` enum has `Fungible = 3` variant (ADR 0031 SMALLINT).
      - `xdr_parser::classify_contract_from_wasm_spec` classifies
        deployed contracts by WASM spec (owner_of/token_uri → Nft,
        decimals/allowance/total_supply → Fungible, otherwise Other).
      - Staging step `staging.rs:419` runs classification at ingest
        time and maps to `ContractType` via `impl From<ContractClassification>`.
      - `write::reclassify_contracts_from_wasm` UPDATEs
        `soroban_contracts.contract_type` when late WASM upload promotes
        a previously-Other contract (task 0118 Phase 2).
      - `write::upsert_assets_soroban` inserts TokenAssetType::Soroban
        rows with `ON CONFLICT (contract_id) WHERE asset_type IN (2,3) DO UPDATE`
        (partial UNIQUE `uidx_assets_soroban`, migration 0005).
      - Partial UNIQUE indexes + ck_assets_identity CHECK enforce the
        asset_type discriminants.

      NOT DONE (real 0120 work):
      - `xdr_parser::detect_assets` at `state.rs:471` **only handles SAC**.
        It ignores non-SAC deployments entirely — no `ExtractedAsset`
        with `asset_type = Soroban` is ever emitted, so the
        `upsert_assets_soroban` write path never receives rows for
        WASM-classified Fungible contracts.
      - `reclassify_contracts_from_wasm` UPDATE does NOT insert a
        missing `assets` row when it promotes `Other → Fungible` on
        a late WASM upload. Contract gets correctly typed; `assets`
        table stays empty for that contract.
      - `ExtractedContractDeployment.metadata` is always `json!({})`
        (state.rs:69) — no name/symbol extraction path exists.
  - date: '2026-04-24'
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped via PR #113 (commit 2bfa683). 6 files changed,
      +856/-10 lines. `detect_assets` extended with `interfaces`
      slice + WASM-based Fungible classification (SAC branch
      unchanged). Late-WASM bridge `insert_assets_from_reclassified_contracts`
      wired into persist tx after `upsert_assets` +
      `reclassify_contracts_from_wasm`, idempotent via NOT EXISTS +
      ON CONFLICT DO NOTHING. Tests: 6 new unit tests in xdr-parser
      (SAC regression, Fungible WASM, NFT WASM, Other WASM,
      dual-interface precedence, missing interface); 2 integration
      tests in indexer including late-WASM 2-ledger bridge with
      replay idempotency. Follow-up 0156 (name/symbol from ContractData)
      spawned.
---

# Indexer: detect Soroban-native tokens (non-SAC)

## Summary

`detect_assets` only emits `ExtractedAsset` rows for SAC deployments.
WASM-based contracts that classify as `ContractType::Fungible`
(SEP-0041 surface: `decimals`, `allowance`, `total_supply`) never produce
an `assets` row. The classification infrastructure and persist layer are
already in place (task 0118 Phase 2 + existing `upsert_assets_soroban`);
this task closes the last gap by wiring the two ends together and
handling the late-WASM-upload case.

## Context

Audit finding F8 (2026-04-10): `contract_type` was binary SAC-vs-other
with no WASM-based classification. Task 0118 (Phase 1 + Phase 2, merged
as PR #104 + PR #110) introduced `classify_contract_from_wasm_spec` and
wired it to `soroban_contracts.contract_type` — but the NFT-focused
scope of 0118 did not touch the `assets` table. 0120 covers the token
side of the same classification pipeline.

**Post-ADR-0027 / ADR-0031 schema:**

- `assets.asset_type SMALLINT` maps to `TokenAssetType` enum:
  `Native=0, Classic=1, Sac=2, Soroban=3`.
- `uidx_assets_soroban ON assets (contract_id) WHERE asset_type IN (2, 3)`
  enforces single asset row per SAC or Soroban contract.
- `ck_assets_identity` CHECK ensures `Soroban` rows have `contract_id`
  set and `issuer_id` NULL.

## Implementation

### 1. Extend `detect_assets` with WASM-based path

`crates/xdr-parser/src/state.rs:471` currently:

```rust
pub fn detect_assets(deployments: &[ExtractedContractDeployment]) -> Vec<ExtractedAsset> {
    // only SAC branch — non-SAC ignored
}
```

Extend signature to take `interfaces` slice so the fn can classify WASM
contracts:

```rust
pub fn detect_assets(
    deployments: &[ExtractedContractDeployment],
    interfaces: &[ExtractedContractInterface],
) -> Vec<ExtractedAsset>
```

Logic for each non-SAC deployment with a `wasm_hash`:

1. Find matching `ExtractedContractInterface` by `wasm_hash`.
2. Call `classify_contract_from_wasm_spec(&iface.functions)`.
3. If verdict is `Fungible`: push `ExtractedAsset { asset_type: TokenAssetType::Soroban, contract_id: Some(...), ... }`.
4. `Nft` and `Other` verdicts: **no assets row** — NFTs live in the
   `nfts` table, `Other` isn't a known token.

Update the single caller in `crates/indexer/src/handler/process.rs:131`:

```rust
let assets = xdr_parser::detect_assets(&deployments, &interfaces);
```

The parser already has access to `all_contract_interfaces` from an
earlier stage (`process.rs:104`).

### 2. Name / symbol best-effort

The parser does NOT currently extract contract name/symbol
(`ExtractedContractDeployment.metadata` is always `json!({})`). For this
task, leave `ExtractedAsset.name = None` and `.symbol = None`;
`upsert_assets_soroban` already COALESCEs NULLs on conflict so a later
population run won't regress existing rows.

**Follow-up backlog task** (spawned from this task): extract name/symbol
from contract's ContractData storage entries at deploy time. Requires
decoding ContractData keys/values from ledger entry changes — adjacent
to existing extraction but not trivial.

### 3. Bridge late-WASM reclassification → assets row

`write::reclassify_contracts_from_wasm` promotes
`soroban_contracts.contract_type Other → Fungible` for existing
contracts when a new WASM upload is observed. Currently no `assets` row
is inserted in that path.

Extend either `reclassify_contracts_from_wasm` or add a sibling step
that runs after it. Concrete SQL pattern:

```sql
INSERT INTO assets (asset_type, contract_id)
SELECT 3, c.id
  FROM soroban_contracts c
 WHERE c.contract_type = 3                  -- Fungible
   AND c.wasm_hash = ANY($1::BYTEA[])       -- hashes classified this tx
   AND NOT EXISTS (
         SELECT 1 FROM assets a
          WHERE a.contract_id = c.id
            AND a.asset_type IN (2, 3)      -- sac, soroban
       )
ON CONFLICT (contract_id) WHERE asset_type IN (2, 3) DO NOTHING;
```

Runs inside the persist tx. Idempotent — replay safe.

### 4. Tests

- **Unit** (`xdr-parser/src/state.rs`): `detect_assets` with a WASM
  deployment + matching interface exposing `decimals` + `transfer` →
  returns `ExtractedAsset { asset_type: Soroban, contract_id: Some(_), .. }`.
- **Unit**: `detect_assets` with NFT deployment (WASM exposing
  `owner_of`) → returns NO assets row (nfts pipeline only).
- **Unit**: `detect_assets` with Other deployment (no standard
  discriminators) → no assets row.
- **Unit**: SAC deployment still produces a Sac asset row (regression
  guard).
- **Integration** (`crates/indexer/tests/persist_integration.rs`):
  synthetic ledger with a WASM deploy + Fungible interface → `assets`
  has exactly one row with `asset_type = 3` and `contract_id` set.
- **Integration**: late-WASM scenario — contract deployed in ledger N
  (asset_type absent), WASM uploaded in ledger N+1 → after second
  ledger's persist, `assets` row exists.

## Acceptance Criteria

- [x] `detect_assets` extended to accept `&[ExtractedContractInterface]`
      and emit Soroban asset rows for `Fungible` classifications.
- [x] Single caller in `handler/process.rs` updated to pass interfaces.
- [x] Non-SAC Fungible contracts deployed in a ledger result in exactly
      one `assets` row with `asset_type = 3` (Soroban) after persist.
- [x] Late WASM upload promoting an existing contract to `Fungible`
      inserts the missing assets row (idempotent, ON CONFLICT DO NOTHING).
- [x] NFT-classified contracts produce NO `assets` row (separation of
      concerns: NFT contracts stay in `nfts`/`nft_ownership` only).
- [x] Existing SAC detection unchanged (regression-guarded by unit test).
- [x] Integration test: deploy+classify → `assets` row, asset_type = 3.
- [x] Integration test: late-WASM upload → `assets` row added.
- [x] Name/symbol left as `None` with follow-up backlog task spawned.

## Design Decisions

### From Plan

1. **Fungible → assets, NFT → nfts.** Strict separation. NFT contracts
   never produce assets rows; assets table holds only fungible-shaped
   assets (Native / Classic / SAC / Soroban-Fungible).

2. **Name/symbol deferred to follow-up.** Extraction requires either
   decoding ContractData ledger entry changes for standard keys
   (`name`, `symbol`) or calling the contract — out of scope for
   detection wiring. `upsert_assets_soroban` already handles NULL
   gracefully via COALESCE on conflict, so a later population run is a
   clean incremental win without schema change.

### Emerged (post-takeover audit)

3. **Extend `detect_assets` signature rather than adding
   `detect_soroban_assets`.** Caller site is a single line in
   `process.rs`; unified fn keeps SAC and Soroban branches adjacent,
   easier to maintain consistency (both paths update on schema changes).

4. **Late-WASM-upload path gets its own SQL, not a detect_assets
   invocation.** By the time `reclassify_contracts_from_wasm` fires,
   the deployment is no longer in-memory as `ExtractedContractDeployment`
   — it's already persisted. Adding a second DB-driven INSERT path is
   cleaner than re-materialising the deployment slice. Idempotency via
   `ON CONFLICT DO NOTHING`.

## Future Work → Backlog

- **Extract token name/symbol from ContractData entries** — spawned as
  [task 0156](../backlog/0156_FEATURE_soroban-token-name-symbol-extraction.md).
  At deploy time, scan LedgerEntryChanges for standard contract-data keys
  (`Symbol("name")`, `Symbol("symbol")`) and populate
  `ExtractedAsset.name`/`.symbol`. Needs reverse-engineering of the
  OpenZeppelin / SDK storage layout. effort-small.

## Implementation Notes

**Files changed (6, +856/-10 lines per PR #113, commit 2bfa683):**

- `crates/xdr-parser/src/state.rs` (+218, -10) — `detect_assets`
  signature extended to take `&[ExtractedContractInterface]`; pre-index
  interfaces by `wasm_hash` (HashMap, amortised O(1)) + cache
  `classify_contract_from_wasm_spec` verdict per hash. SAC branch
  unchanged. Non-SAC with Fungible verdict emits `ExtractedAsset {
asset_type: Soroban, contract_id: Some(_), .. }`. NFT/Other verdicts
  produce no asset row.
- `crates/indexer/src/handler/process.rs` (+7) — single caller updated
  to pass `&interfaces`.
- `crates/indexer/src/handler/persist/write.rs` (+74) —
  `insert_assets_from_reclassified_contracts`: SQL `INSERT ... SELECT
FROM soroban_contracts WHERE contract_type = 3 AND wasm_hash = ANY($1)
AND NOT EXISTS (SELECT 1 FROM assets WHERE contract_id = ...) ON
CONFLICT (contract_id) WHERE asset_type IN (2, 3) DO NOTHING`.
- `crates/indexer/src/handler/persist/mod.rs` (+4) — call site wired
  after `upsert_assets` + `reclassify_contracts_from_wasm` so
  `soroban_contracts.contract_type` is authoritative and duplicates are
  avoided.
- `crates/indexer/tests/persist_integration.rs` (+445) — 2 new
  integration tests: same-ledger detect+classify → assets row;
  late-WASM 2-ledger path (deploy in N, WASM in N+1) → assets row
  inserted after second persist. Replay idempotency verified by
  re-running ledger N+1 persist.
- `lore/1-tasks/backlog/0156_FEATURE_soroban-token-name-symbol-extraction.md`
  (+118) — spawned follow-up.

**Tests added (8):**

- Unit (`xdr-parser/src/state.rs`): `detect_assets_sac_regression`,
  `detect_assets_fungible_from_wasm`, `detect_assets_nft_wasm_no_row`,
  `detect_assets_other_wasm_no_row`, `detect_assets_dual_interface_precedence`,
  `detect_assets_missing_interface_skipped`.
- Integration (`indexer/tests/persist_integration.rs`):
  deploy-and-classify end-to-end; `late_wasm_upload_backfills_assets_row`
  (same fn serves as replay idempotency check — re-run second-ledger
  persist, assert single row).
