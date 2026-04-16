---
id: '0120'
title: 'Indexer: detect Soroban-native tokens (non-SAC)'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0027', '0049', '0104', '0124']
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
  - date: '2026-04-16'
    status: done
    who: FilipDz
    note: >
      DB-only detection via CTE after metadata merge. is_sep41_compliant helper
      in state.rs, detect_soroban_tokens_from_metadata SQL in soroban.rs,
      wired as step 8.5 in persist.rs. 3 unit + 5 integration tests.
      Name/symbol deferred to task 0124.
---

# Indexer: detect Soroban-native tokens (non-SAC)

## Summary

`contract_type` classification is binary: SAC = "token", everything else = "other".
WASM-based contracts implementing the SEP-0041 token interface are never detected as
tokens and never added to the `tokens` table.

## Context

The `wasm_interface_metadata` staging table already stores function signatures for deployed
contracts. A contract implementing `transfer`, `balance`, `decimals`, `name`, `symbol` is
almost certainly a token. This data is available — it just needs to be used for
classification.

## Implementation

1. After contract interface metadata is merged, check function signatures against SEP-0041
   required functions (`transfer`, `balance`, `decimals`, `name`, `symbol`).
2. If a contract matches, classify `contract_type = "token"` and create a `tokens` entry
   with `asset_type = "soroban"`.
3. Populate token `name` and `symbol` from contract metadata where available.
4. Update `ON CONFLICT` logic in token upsert to handle `asset_type = 'soroban'` correctly
   (addresses audit finding F12).

## Acceptance Criteria

- [x] Contracts implementing SEP-0041 interface are classified as `contract_type = "token"`
- [x] Corresponding `tokens` row created with `asset_type = "soroban"` and `contract_id`
- [ ] ~~Token name/symbol populated from contract metadata when available~~ — deferred to task 0124 (metadata contains function signatures, not actual values)
- [x] Existing SAC token detection unchanged
- [x] Test: WASM contract with SEP-0041 functions detected as token
