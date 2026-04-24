---
id: '0160'
title: 'BUG: SAC deployments never land in assets — missing underlying asset_code/issuer extraction'
type: BUG
status: active
related_adr: ['0023', '0024', '0036', '0037']
related_tasks: ['0120', '0124', '0154']
tags: [priority-high, effort-small, layer-indexer, layer-xdr-parser, audit-gap]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/indexer/src/handler/persist/write.rs
  - crates/indexer/src/handler/persist/staging.rs
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
      Frontmatter ADR refs updated — dropped superseded `0027`, added
      `0037` (current authoritative schema) and `0024` (BYTEA(32) hash
      constraint for assets). No scope change.
---

# BUG: SAC deployments never land in assets — missing underlying asset_code/issuer extraction

## Summary

`xdr_parser::detect_assets` emits an `ExtractedAsset` row for every SAC
deployment with `asset_type = Sac` but `asset_code = None` and
`issuer_address = None`. The staging path `upsert_assets_classic_like`
then filters those rows out because both fields are required for the
classic-like INSERT. Net effect: **SAC deployments silently dropped, no
row ever lands in `assets`.** Gap has existed since initial
implementation; 0120 (detection wiring) and 0154 (tokens→assets rename)
both inherited it unchanged.

## Context

Reproduction:

- `crates/xdr-parser/src/state.rs:513-522` — SAC branch pushes
  `ExtractedAsset` with `asset_code: None`, `issuer_address: None`.
- `crates/indexer/src/handler/persist/write.rs:1095-1101` —
  `upsert_assets_classic_like` does `let Some(code) = r.asset_code
else { continue; }` and the analogous issuer guard. SAC rows fall
  through both.
- `ck_assets_identity` for `asset_type = 2` (Sac) requires
  `asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id
IS NOT NULL`. The staging skip is correct defensive behaviour —
  without it the INSERT would violate the CHECK. The real fix is
  upstream: populate the fields in the parser.

Underlying data is present in the XDR. `is_sac_from_data` already
peels `data.val.value.executable` and checks
`type == "stellar_asset"`. The same subtree carries the wrapped
`AssetCode4 / AssetCode12` + `issuer` (G-address) for the classic
asset that backs the SAC. `extract_contract_deployments` just ignores
it today.

Audit note: 2026-04-10 pipeline audit §5.1 tables flagged
`tokens.asset_code` / `tokens.issuer_address` as nullable only for
Soroban-native assets — implicit expectation that SAC/classic have
them populated, but no task captured the extraction work.

## Implementation

### 1. Extend `extract_contract_deployments` to read SAC underlying asset

`crates/xdr-parser/src/state.rs:42-72` — when `is_sac` is true, peel
the `executable.stellar_asset` subtree for the wrapped classic asset:

- `asset_type` = `native` | `credit_alphanum4` | `credit_alphanum12`
- `code` (absent for native)
- `issuer` (G-address, absent for native)

Shape matches the trustline path in `extract_account_states`
(`state.rs:225-246`) so the JSON layout stays consistent.

Add two fields to `ExtractedContractDeployment` (or extend
`metadata: serde_json::Value` — decide at implementation time; prefer
typed fields for SAC since they are part of identity, not free-form
metadata):

```rust
pub struct ExtractedContractDeployment {
    // existing fields
    pub sac_asset_code: Option<String>,
    pub sac_asset_issuer: Option<String>,
}
```

### 2. Thread through `detect_assets`

`state.rs:513-522` SAC branch populates the new fields:

```rust
assets.push(ExtractedAsset {
    asset_type: TokenAssetType::Sac,
    asset_code: deployment.sac_asset_code.clone(),
    issuer_address: deployment.sac_asset_issuer.clone(),
    contract_id: Some(deployment.contract_id.clone()),
    ...
});
```

For XLM-wrapped SAC (native): `asset_code` + `issuer` are both `None`
but `contract_id` is set. This is a known edge case — XLM-SAC is a
singleton, typically bootstrapped once on the network. Either:

- (a) leave as `None`/`None`, update staging to accept SAC with all
  three identity fields NULL provided there's a separate partial
  UNIQUE covering `(contract_id) WHERE asset_type = 2 AND asset_code
IS NULL` — but this conflicts with `ck_assets_identity` for
  `asset_type = 2`.
- (b) treat XLM-SAC as `asset_type = Soroban` (go through
  `upsert_assets_soroban`, identity by `contract_id`) and document
  the override.
- (c) synthesise `asset_code = "XLM"` + fixed XLM issuer sentinel.

Pick (b) or (c) at implementation time; update `ck_assets_identity`
and/or parser accordingly. Note in this task which one landed.

### 3. Tests

- **Unit** (`xdr-parser`): SAC deployment for credit_alphanum4 asset
  → `ExtractedAsset { asset_code: Some("USDC"), issuer_address:
Some("G..."), contract_id: Some(...), asset_type: Sac, .. }`.
- **Unit**: SAC for credit_alphanum12 similarly.
- **Unit**: XLM-SAC (native-wrapping) — documents the chosen edge-case
  handling.
- **Integration** (`persist_integration.rs`): synthetic ledger with a
  SAC deployment → `assets` row exists with non-NULL `asset_code`,
  `issuer_id`, `contract_id`.

## Acceptance Criteria

- [ ] `extract_contract_deployments` extracts wrapped classic asset
      (code + issuer) from SAC `executable.stellar_asset` subtree.
- [ ] `detect_assets` populates `ExtractedAsset.asset_code` and
      `.issuer_address` for SAC deployments.
- [ ] XLM-SAC (native-wrapping) edge case handled and documented.
- [ ] Unit test coverage per §3.
- [ ] Integration test: SAC deploy → `assets` row with all three
      identity fields non-NULL (for credit) or per chosen XLM-SAC
      strategy (for native).
- [ ] `upsert_assets_classic_like` no longer silently drops SAC rows
      that now carry code + issuer.
- [ ] Existing SAC regression tests (`persist_integration.rs`)
      updated to assert populated identity fields.

## Notes

Audit gap. Priority high because it affects asset-level completeness
of the entire explorer — classic/SAC tokens are the dominant population
on mainnet and the `assets` table is empty without this fix.
