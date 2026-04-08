---
id: '0104'
title: 'Persist contract interface metadata via wasm_hash→contract_id join'
type: FEATURE
status: completed
related_adr: ['0004']
related_tasks: ['0026', '0029']
tags: [priority-medium, effort-small, layer-indexing, rust]
milestone: 1
links: []
history:
  - date: 2026-04-07
    status: backlog
    who: FilipDz
    note: 'Spawned from 0029 future work. Contract interface metadata (function signatures from WASM) cannot be stored correctly because ExtractedContractInterface only has wasm_hash, not contract_id.'
  - date: 2026-04-08
    status: completed
    who: FilipDz
    note: >
      Implemented wasm_hash→contract_id join for interface metadata persistence.
      3 files changed. 2 new integration tests passing (9 total in db crate).
      Migration 0008 adds partial index on wasm_hash. PR #78.
---

# Persist contract interface metadata via wasm_hash→contract_id join

## Summary

Task 0026 extracts contract function signatures from WASM bytecode (`contractspecv0` section) into `ExtractedContractInterface`, but this struct only carries `wasm_hash` — not `contract_id`. Since `soroban_contracts` is keyed by `contract_id`, we need a way to join wasm_hash back to contract_id before storing the metadata.

## Context

During ledger processing (task 0029), step 7 (contract interface metadata) is currently skipped with a TODO. The problem:

- `extract_contract_interfaces()` (task 0026) parses WASM from `ContractCodeEntry` in ledger entry changes
- It produces `ExtractedContractInterface { wasm_hash, functions, wasm_byte_len }`
- But `soroban_contracts` PK is `contract_id` (the Stellar contract address, e.g. `CXXX...`)
- Multiple contracts can share the same `wasm_hash` (same bytecode deployed multiple times)
- There's no direct mapping from wasm_hash → contract_id available at interface extraction time

## Implementation Plan

1. In `persist_ledger()`, process contract interfaces **after** contract deployments (step 8) so that `soroban_contracts` rows already exist
2. For each `ExtractedContractInterface`, query `soroban_contracts` within the transaction to find all rows where `wasm_hash` matches
3. Update `metadata` JSONB on each matching contract with the extracted function signatures
4. If no matching contracts exist yet (edge case: WASM uploaded but no contract deployed in this ledger), store the interface keyed by `wasm_hash` in a staging table or skip (TBD)

## Acceptance Criteria

- [x] Contract function signatures are stored in `soroban_contracts.metadata` JSONB
- [x] Multiple contracts sharing the same wasm_hash all get metadata populated
- [x] Interface extraction works correctly when deployment and WASM upload happen in the same ledger
- [x] Replay-safe: re-processing a ledger does not corrupt metadata

## Implementation Notes

- **Migration 0008**: partial index `idx_contracts_wasm_hash` on `soroban_contracts(wasm_hash) WHERE wasm_hash IS NOT NULL`
- **`update_contract_interfaces_by_wasm_hash()`** in `crates/db/src/soroban.rs`: UPDATE with JSONB `||` merge, returns rows_affected count
- **`persist_ledger()` step reorder**: contract deployments (old step 8) moved to step 7; interface metadata now step 8, runs after deployments so `wasm_hash` is populated
- Warns (no error) when WASM uploaded without deployment in same ledger — metadata will be applied when the contract is eventually deployed and re-indexed

## Design Decisions

### From Plan

1. **Join via DB query on wasm_hash**: as specified in task spec, query `soroban_contracts` for all rows matching `wasm_hash` after deployments are upserted.

### Emerged

2. **Step reorder (7↔8) instead of two-pass**: rather than a separate pass, swapped the deployment and interface steps so the existing single-pass flow works. Simpler than the task spec's suggestion of a staging table.
3. **Warn-and-skip for orphan WASM**: task spec suggested a staging table (TBD). Chose to warn and skip — the metadata will be applied on re-index after the contract is deployed. Avoids new table complexity for a rare edge case.
