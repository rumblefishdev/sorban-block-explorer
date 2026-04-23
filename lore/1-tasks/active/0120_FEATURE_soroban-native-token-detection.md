---
id: '0120'
title: 'Indexer: detect Soroban-native tokens (non-SAC) and persist to tokens table'
type: FEATURE
status: active
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
      SCOPE REWRITE. Original spec (classify + insert `tokens` row)
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
      - `write::upsert_tokens_soroban` inserts TokenAssetType::Soroban
        rows with `ON CONFLICT (contract_id) WHERE asset_type IN (2,3) DO UPDATE`
        (partial UNIQUE `uidx_tokens_soroban`, migration 0005).
      - Partial UNIQUE indexes + ck_tokens_identity CHECK enforce the
        asset_type discriminants.

      NOT DONE (real 0120 work):
      - `xdr_parser::detect_tokens` at `state.rs:471` **only handles SAC**.
        It ignores non-SAC deployments entirely — no `ExtractedToken`
        with `asset_type = Soroban` is ever emitted, so the
        `upsert_tokens_soroban` write path never receives rows for
        WASM-classified Fungible contracts.
      - `reclassify_contracts_from_wasm` UPDATE does NOT insert a
        missing `tokens` row when it promotes `Other → Fungible` on
        a late WASM upload. Contract gets correctly typed; `tokens`
        table stays empty for that contract.
      - `ExtractedContractDeployment.metadata` is always `json!({})`
        (state.rs:69) — no name/symbol extraction path exists.
---

# Indexer: detect Soroban-native tokens (non-SAC)

## Summary

`detect_tokens` only emits `ExtractedToken` rows for SAC deployments.
WASM-based contracts that classify as `ContractType::Fungible`
(SEP-0041 surface: `decimals`, `allowance`, `total_supply`) never produce
a `tokens` row. The classification infrastructure and persist layer are
already in place (task 0118 Phase 2 + existing `upsert_tokens_soroban`);
this task closes the last gap by wiring the two ends together and
handling the late-WASM-upload case.

## Context

Audit finding F8 (2026-04-10): `contract_type` was binary SAC-vs-other
with no WASM-based classification. Task 0118 (Phase 1 + Phase 2, merged
as PR #104 + PR #110) introduced `classify_contract_from_wasm_spec` and
wired it to `soroban_contracts.contract_type` — but the NFT-focused
scope of 0118 did not touch the `tokens` table. 0120 covers the token
side of the same classification pipeline.

**Post-ADR-0027 / ADR-0031 schema:**

- `tokens.asset_type SMALLINT` maps to `TokenAssetType` enum:
  `Native=0, Classic=1, Sac=2, Soroban=3`.
- `uidx_tokens_soroban ON tokens (contract_id) WHERE asset_type IN (2, 3)`
  enforces single token row per SAC or Soroban contract.
- `ck_tokens_identity` CHECK ensures `Soroban` rows have `contract_id`
  set and `issuer_id` NULL.

## Implementation

### 1. Extend `detect_tokens` with WASM-based path

`crates/xdr-parser/src/state.rs:471` currently:

```rust
pub fn detect_tokens(deployments: &[ExtractedContractDeployment]) -> Vec<ExtractedToken> {
    // only SAC branch — non-SAC ignored
}
```

Extend signature to take `interfaces` slice so the fn can classify WASM
contracts:

```rust
pub fn detect_tokens(
    deployments: &[ExtractedContractDeployment],
    interfaces: &[ExtractedContractInterface],
) -> Vec<ExtractedToken>
```

Logic for each non-SAC deployment with a `wasm_hash`:

1. Find matching `ExtractedContractInterface` by `wasm_hash`.
2. Call `classify_contract_from_wasm_spec(&iface.functions)`.
3. If verdict is `Fungible`: push `ExtractedToken { asset_type: TokenAssetType::Soroban, contract_id: Some(...), ... }`.
4. `Nft` and `Other` verdicts: **no tokens row** — NFTs live in the
   `nfts` table, `Other` isn't a known token.

Update the single caller in `crates/indexer/src/handler/process.rs:131`:

```rust
let tokens = xdr_parser::detect_tokens(&deployments, &interfaces);
```

The parser already has access to `all_contract_interfaces` from an
earlier stage (`process.rs:104`).

### 2. Name / symbol best-effort

The parser does NOT currently extract contract name/symbol
(`ExtractedContractDeployment.metadata` is always `json!({})`). For this
task, leave `ExtractedToken.name = None` and `.symbol = None`;
`upsert_tokens_soroban` already COALESCEs NULLs on conflict so a later
population run won't regress existing rows.

**Follow-up backlog task** (spawned from this task): extract name/symbol
from contract's ContractData storage entries at deploy time. Requires
decoding ContractData keys/values from ledger entry changes — adjacent
to existing extraction but not trivial.

### 3. Bridge late-WASM reclassification → tokens row

`write::reclassify_contracts_from_wasm` promotes
`soroban_contracts.contract_type Other → Fungible` for existing
contracts when a new WASM upload is observed. Currently no `tokens` row
is inserted in that path.

Extend either `reclassify_contracts_from_wasm` or add a sibling step
that runs after it. Concrete SQL pattern:

```sql
INSERT INTO tokens (asset_type, contract_id)
SELECT 3, c.id
  FROM soroban_contracts c
 WHERE c.contract_type = 3                  -- Fungible
   AND c.wasm_hash = ANY($1::BYTEA[])       -- hashes classified this tx
   AND NOT EXISTS (
         SELECT 1 FROM tokens t
          WHERE t.contract_id = c.id
            AND t.asset_type IN (2, 3)      -- sac, soroban
       )
ON CONFLICT (contract_id) WHERE asset_type IN (2, 3) DO NOTHING;
```

Runs inside the persist tx. Idempotent — replay safe.

### 4. Tests

- **Unit** (`xdr-parser/src/state.rs`): `detect_tokens` with a WASM
  deployment + matching interface exposing `decimals` + `transfer` →
  returns `ExtractedToken { asset_type: Soroban, contract_id: Some(_), .. }`.
- **Unit**: `detect_tokens` with NFT deployment (WASM exposing
  `owner_of`) → returns NO tokens row (nfts pipeline only).
- **Unit**: `detect_tokens` with Other deployment (no standard
  discriminators) → no tokens row.
- **Unit**: SAC deployment still produces a Sac token row (regression
  guard).
- **Integration** (`crates/indexer/tests/persist_integration.rs`):
  synthetic ledger with a WASM deploy + Fungible interface → `tokens`
  has exactly one row with `asset_type = 3` and `contract_id` set.
- **Integration**: late-WASM scenario — contract deployed in ledger N
  (asset_type absent), WASM uploaded in ledger N+1 → after second
  ledger's persist, `tokens` row exists.

## Acceptance Criteria

- [ ] `detect_tokens` extended to accept `&[ExtractedContractInterface]`
      and emit Soroban token rows for `Fungible` classifications.
- [ ] Single caller in `handler/process.rs` updated to pass interfaces.
- [ ] Non-SAC Fungible contracts deployed in a ledger result in exactly
      one `tokens` row with `asset_type = 3` (Soroban) after persist.
- [ ] Late WASM upload promoting an existing contract to `Fungible`
      inserts the missing tokens row (idempotent, ON CONFLICT DO NOTHING).
- [ ] NFT-classified contracts produce NO `tokens` row (separation of
      concerns: NFT contracts stay in `nfts`/`nft_ownership` only).
- [ ] Existing SAC detection unchanged (regression-guarded by unit test).
- [ ] Integration test: deploy+classify → `tokens` row, asset_type = 3.
- [ ] Integration test: late-WASM upload → `tokens` row added.
- [ ] Name/symbol left as `None` with follow-up backlog task spawned.

## Design Decisions

### From Plan

1. **Fungible → tokens, NFT → nfts.** Strict separation. NFT contracts
   never produce tokens rows; tokens table holds only fungible-shaped
   assets (Native / Classic / SAC / Soroban-Fungible).

2. **Name/symbol deferred to follow-up.** Extraction requires either
   decoding ContractData ledger entry changes for standard keys
   (`name`, `symbol`) or calling the contract — out of scope for
   detection wiring. `upsert_tokens_soroban` already handles NULL
   gracefully via COALESCE on conflict, so a later population run is a
   clean incremental win without schema change.

### Emerged (post-takeover audit)

3. **Extend `detect_tokens` signature rather than adding
   `detect_soroban_tokens`.** Caller site is a single line in
   `process.rs`; unified fn keeps SAC and Soroban branches adjacent,
   easier to maintain consistency (both paths update on schema changes).

4. **Late-WASM-upload path gets its own SQL, not a detect_tokens
   invocation.** By the time `reclassify_contracts_from_wasm` fires,
   the deployment is no longer in-memory as `ExtractedContractDeployment`
   — it's already persisted. Adding a second DB-driven INSERT path is
   cleaner than re-materialising the deployment slice. Idempotency via
   `ON CONFLICT DO NOTHING`.

## Future Work → Backlog

- **Extract token name/symbol from ContractData entries** (spawned
  follow-up task): at deploy time, scan LedgerEntryChanges for standard
  contract-data keys (`Symbol("name")`, `Symbol("symbol")`) and populate
  `ExtractedToken.name`/`.symbol`. Needs reverse-engineering of the
  OpenZeppelin / SDK storage layout. Eff effort-small.
