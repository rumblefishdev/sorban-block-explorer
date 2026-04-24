---
id: '0164'
title: 'Indexer: correlate multiple SAC deployments per tx via preimage hash'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0160']
tags: [priority-low, effort-medium, layer-indexer, layer-xdr-parser, audit-gap]
milestone: 1
links:
  - crates/indexer/src/handler/process.rs
  - crates/xdr-parser/src/state.rs
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0160 future work. 0160 keyed the SAC identity
      map by tx_hash under the assumption of at most one SAC
      CreateContract per tx. Theoretically possible to deploy >1
      SAC in one tx (bundled ops); if observed on mainnet the
      current map would attribute the wrong asset to a contract.
      Fix by switching to preimage-hash ↔ contract_id correlation.
---

# Indexer: correlate multiple SAC deployments per tx via preimage hash

## Summary

Replace the tx_hash → `SacAssetIdentity` correlation map from task
0160 with a preimage-hash → contract_id correlation so that
transactions deploying more than one SAC still route each asset
identity to the correct contract.

## Context

0160 assumes at most one `CreateContract FromAsset` op per tx. A
tx with N SAC deploys would today have only the first identity
attributed, and later ContractInstance entries in the same tx
would fall through to the XLM-SAC sentinel path (wrong classic
asset label).

Deriving contract_id from preimage requires:
`contract_id = SHA256(network_id || XDR-serialize(ContractIdPreimage))`
matched against the network passphrase hash. Then the map key
becomes the contract_id itself, unambiguous even within a single
tx.

## Implementation

1. Confirm the population — scan mainnet for txs with >1
   CreateContract op where any have `FromAsset` preimage. Decide
   priority based on count.
2. In `xdr-parser`, add a helper computing contract_id from
   `ContractIdPreimage` + network passphrase (same derivation
   stellar-core uses). Unit test against known mainnet contract
   IDs.
3. Update `process.rs` correlation map keyed by `contract_id`
   instead of `tx_hash`. `extract_contract_deployments` takes the
   map by contract_id lookup.
4. Tests: tx with 2 SAC deploys (USDC, EURC) → both assets rows
   populated with correct identities.

## Acceptance Criteria

- [ ] Mainnet measurement documented (multi-SAC-per-tx count).
- [ ] Preimage hash → contract_id derivation implemented + unit
      tested against known mainnet SAC deployment.
- [ ] `process.rs` correlation map keyed by contract_id.
- [ ] Integration test: tx with 2 FromAsset deploys → 2 distinct
      `assets` rows with correct identities.
- [ ] Existing 0160 single-SAC tests unchanged.

## Notes

Low priority until metrics justify. Effort medium because the
preimage hash derivation is non-trivial (XDR serialize + SHA256 +
network_id).
