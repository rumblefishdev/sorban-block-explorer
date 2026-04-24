---
id: '0163'
title: 'Indexer: detect factory-deployed SACs from CreateContractHostFn auth entries'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0160']
tags: [priority-low, effort-small, layer-indexer, layer-xdr-parser, audit-gap]
milestone: 1
links:
  - crates/xdr-parser/src/invocation.rs
  - crates/xdr-parser/src/state.rs
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0160 future work. 0160 covered SACs deployed via
      top-level `CreateContract` operations only. Contracts (e.g.
      Soroswap factory patterns) that deploy SACs from inner
      invocations show up in `SorobanAuthorizedFunction::CreateContractHostFn`
      in the auth entry path (`invocation.rs:248`) which 0160 does
      not correlate. Coverage gap flagged; observability first, fix
      if the population is non-trivial.
---

# Indexer: detect factory-deployed SACs from CreateContractHostFn auth entries

## Summary

Extend SAC asset identity extraction (task 0160) to cover SACs
deployed from inside a contract (factory pattern) rather than from a
top-level `CreateContract` operation. Factory-deployed SACs are
invisible to 0160's top-level correlation today.

## Context

Task 0160 reads
`CreateContractArgs.contract_id_preimage.FromAsset(Asset)` from each
tx's top-level `ExtractedOperation.details` and threads the identity
to `detect_assets` via a tx_hash → `SacAssetIdentity` map.
`invocation.rs:248` already parses the same `CreateContractArgs`
from auth entries (`SorobanAuthorizedFunction::CreateContractHostFn`)
but the preimage is dropped there, and the auth-entry path does not
feed the 0160 correlation map.

Factory-deployed SACs therefore produce no `assets` row today — the
ContractInstance ledger entry still appears (so `soroban_contracts`
gets the row), but `sac_asset_code` / `sac_asset_issuer` stay `None`
and `detect_assets` falls back to the XLM-SAC sentinel, mislabelling
the wrapper as native.

## Implementation

1. Measure first. Query mainnet `soroban_contracts` for
   `is_sac = true` rows whose `(asset_code, issuer_id)` in `assets`
   resolves to the XLM-SAC sentinel. Non-trivial count = real
   population.
2. Extend `invocation.rs:248` (and `:260` for V2) to include
   `contractIdPreimage` in the invocation JSON, same shape as
   `operation.rs`.
3. Either extend `extract_sac_asset_from_create_contract` to also
   walk invocations, or add a sibling fn
   `extract_sac_assets_from_invocations` and merge results in
   `process.rs` before the deployment loop.
4. Add unit + integration tests covering a factory-deployed SAC.

## Acceptance Criteria

- [ ] Mainnet measurement documented (sentinel-issuer SAC count).
- [ ] Auth-entry `CreateContractHostFn` preimage extracted.
- [ ] `process.rs` correlation map merges top-level + auth-entry
      sources.
- [ ] Factory-deployed SAC → `assets` row with real
      `asset_code` / `issuer_id`.
- [ ] Existing 0160 tests unchanged (regression).

## Notes

Low priority until metrics show the gap matters. Keep scope narrow
— pure parser extension, no schema change, no migration.
