---
id: '0120'
title: 'Indexer: detect Soroban-native tokens (non-SAC)'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0027', '0049', '0104']
tags: [priority-medium, effort-medium, layer-indexer, audit-F8]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F8 (MEDIUM severity).'
  - date: '2026-04-15'
    status: active
    who: FilipDz
    note: 'Activated for implementation'
---

# Indexer: detect Soroban-native tokens (non-SAC)

## Summary

`contract_type` classification is binary: SAC = "token", everything else = "other".
WASM-based contracts implementing the SEP-0041 token interface are never detected as
tokens and never added to the `assets` table.

## Context

The `wasm_interface_metadata` staging table already stores function signatures for deployed
contracts. A contract implementing `transfer`, `balance`, `decimals`, `name`, `symbol` is
almost certainly a token. This data is available — it just needs to be used for
classification.

## Implementation

1. After contract interface metadata is merged, check function signatures against SEP-0041
   required functions (`transfer`, `balance`, `decimals`, `name`, `symbol`).
2. If a contract matches, classify `contract_type = "token"` and create an `assets` entry
   with `asset_type = "soroban"`.
3. Populate asset `name` and `symbol` from contract metadata where available.
4. Update `ON CONFLICT` logic in asset upsert to handle `asset_type = 'soroban'` correctly
   (addresses audit finding F12).

## Acceptance Criteria

- [ ] Contracts implementing SEP-0041 interface are classified as `contract_type = "token"`
- [ ] Corresponding `assets` row created with `asset_type = "soroban"` and `contract_id`
- [ ] Token name/symbol populated from contract metadata when available
- [ ] Existing SAC token detection unchanged
- [ ] Test: WASM contract with SEP-0041 functions detected as token
