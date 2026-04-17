---
id: '0104'
title: 'Persist contract interface metadata via wasm_hash→contract_id join'
type: FEATURE
status: completed
related_adr: ['0004', '0012']
related_tasks: ['0026', '0029', '0140']
tags:
  [priority-medium, effort-small, layer-indexing, rust, superseded-by-adr-0012]
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
      5 files changed (+migration 0009 staging table). 3 new integration tests.
      Migration 0008: partial index on wasm_hash. Migration 0009: wasm_interface_metadata
      staging table for 2-ledger install+deploy pattern. PR #78.
  - date: '2026-04-17'
    status: completed
    who: stkrolikiewicz
    note: >
      Flagged per task 0140 audit — implementation pattern superseded by ADR 0012
      (zero-upsert schema, insert-only history tables, S3 offload, activity
      projections, created_at partitioning). Body retained as historical record;
      do not use as reference for new work — consult ADR 0012 instead.
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
4. If no matching contracts exist yet (WASM uploaded before contract deployed), store the interface keyed by `wasm_hash` in a `wasm_interface_metadata` staging table and apply it when the contract deployment is upserted in a later ledger

## Acceptance Criteria

- [x] Contract function signatures are stored in `soroban_contracts.metadata` JSONB
- [x] Multiple contracts sharing the same wasm_hash all get metadata populated
- [x] Interface extraction works correctly when deployment and WASM upload happen in the same ledger
- [x] Replay-safe: re-processing a ledger does not corrupt metadata

## Implementation Notes

- **Migration 0008**: partial index `idx_contracts_wasm_hash` on `soroban_contracts(wasm_hash) WHERE wasm_hash IS NOT NULL`
- **Migration 0009**: `wasm_interface_metadata(wasm_hash PK, metadata JSONB)` — staging table for the 2-ledger deploy pattern
- **`update_contract_interfaces_by_wasm_hash()`** in `crates/db/src/soroban.rs`: UPDATE with JSONB `||` merge
- **`upsert_wasm_interface_metadata()`** in `crates/db/src/soroban.rs`: upsert into staging table, ON CONFLICT overwrites (WASM bytecode is immutable)
- **`upsert_contract_deployment()`**: after the main upsert, applies any staged interface metadata via a JOIN on `wasm_interface_metadata` — handles contracts deployed in a later ledger than their WASM upload
- **`persist_ledger()` step 8**: dual-path — always upserts to staging AND applies directly if contracts exist; `upsert_contract_deployment()` picks up the staging data for cross-ledger flows

## Design Decisions

### From Plan

1. **Join via DB query on wasm_hash**: as specified in task spec, query `soroban_contracts` for all rows matching `wasm_hash` after deployments are upserted.

### Emerged

2. **Step reorder (7↔8) instead of two-pass**: rather than a separate pass, swapped the deployment and interface steps so the existing single-pass flow works.
3. **Staging table for 2-ledger install+deploy** (supersedes original warn-and-skip): Soroban's `InvokeHostFunction(UploadContractWasm)` and `InvokeHostFunction(CreateContract)` happen in separate ledgers. `ExtractedContractInterface` is only emitted from `ContractCodeEntry` (WASM upload ledger), so by the time the contract row exists the interface data has already been processed. For a historical backfill processing 50M+ ledgers sequentially, warn-and-skip means any 2-step deployed contract permanently loses its metadata. The staging table (`wasm_interface_metadata`) persists interface data by `wasm_hash` so `upsert_contract_deployment()` can apply it retroactively in the deployment ledger without a re-index.
4. **`wasm_interface_metadata` rows are permanent**: WASM bytecode is immutable on-chain; a given `wasm_hash` always has the same interface. Staging rows can be left indefinitely and serve any future contract deployments reusing the same WASM.
