---
id: '0104'
title: 'Persist contract interface metadata via wasm_hash→contract_id join'
type: FEATURE
status: backlog
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

- [ ] Contract function signatures are stored in `soroban_contracts.metadata` JSONB
- [ ] Multiple contracts sharing the same wasm_hash all get metadata populated
- [ ] Interface extraction works correctly when deployment and WASM upload happen in the same ledger
- [ ] Replay-safe: re-processing a ledger does not corrupt metadata
