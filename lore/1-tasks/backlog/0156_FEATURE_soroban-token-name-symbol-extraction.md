---
id: '0156'
title: 'Indexer: extract Soroban token name/symbol from ContractData at deploy time'
type: FEATURE
status: backlog
related_adr: ['0027', '0031']
related_tasks: ['0120', '0124']
tags: [priority-medium, effort-small, layer-indexer]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/ledger_entry_changes.rs
  - crates/indexer/src/handler/persist/write.rs
history:
  - date: '2026-04-23'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0120 future work. 0120 wires Soroban token detection
      but leaves `ExtractedToken.name` / `.symbol` / `.total_supply` as
      `None`. `upsert_tokens_soroban` COALESCE-es NULL so a later run
      can populate without regressing existing rows.
---

# Indexer: extract Soroban token name/symbol from ContractData at deploy time

## Summary

After 0120, Soroban-native (WASM-based) `Fungible` contracts produce a
`tokens` row at deploy time with `asset_type = Soroban` and `contract_id`
set, but `name` / `symbol` / `total_supply` are NULL. The explorer UI
therefore shows these tokens without a human-readable label. This task
populates those fields at ingest time by reading the contract's
persistent storage entries emitted in the same ledger as the deployment.

## Context

Parent task 0120 (merged) covers the **detection + classification** side
of Soroban token handling. It deliberately defers metadata population to
avoid scope creep while the classification + persist wiring lands.

Sibling task 0124 (backlog) addresses a different enrichment path: a
scheduled Lambda that scans `tokens WHERE metadata IS NULL` and fetches
SEP-1 TOML from issuer home_domains. That Lambda is the right fit for
**classic / SAC** tokens whose metadata is off-chain.

For **Soroban-native** tokens the metadata is **on-chain**: the standard
OpenZeppelin / SDK pattern stores `name`, `symbol`, `decimals` as
persistent ContractData entries keyed by `Symbol("name")`, etc. These
entries show up in the ledger as `ExtractedLedgerEntryChange` records
with `entry_type = "contract_data"` during the contract's init
transaction. This task reads them inline instead of spawning a call
against the chain.

## Implementation

### 1. Extend `ExtractedContractDeployment.metadata`

`crates/xdr-parser/src/state.rs:69` currently writes `json!({})`. Extend
`extract_contract_deployments` to scan adjacent contract_data changes
for standard storage keys and populate:

```rust
metadata: json!({
    "name": <Symbol("name") value, if present>,
    "symbol": <Symbol("symbol") value, if present>,
    "decimals": <Symbol("decimals") value, if present>,
})
```

Keys and decoding follow the OpenZeppelin Stellar contracts
library's `FungibleToken` reference implementation (see classification
code's references).

### 2. Thread name/symbol into `ExtractedToken`

Update `detect_tokens` (task 0120) so the Fungible branch reads
`deployment.metadata["name"]` / `["symbol"]` and places them on the
emitted `ExtractedToken` row. `upsert_tokens_soroban` already handles
partial data via `COALESCE(EXCLUDED.name, tokens.name)`.

### 3. Bridge path — scheduled follow-up, not same tx

Late-WASM contracts (the bridge path in 0120) were deployed in an
earlier ledger; their ContractData changes are not in-memory during the
reclassification. Two options:

**Option A (preferred):** on the next ledger that touches the contract
(any invocation), scan the changes for `contract_data` keys on that
contract id and backfill via `UPDATE tokens SET name = COALESCE(name, …)`.

**Option B:** extend 0124's scheduled enrichment Lambda to decode
on-chain storage for Soroban tokens in addition to SEP-1 TOML. Keeps
the indexer hot path lean.

Decision deferred to implementation; documenting both for the author.

### 4. Tests

- **Unit** (`state.rs`): deployment with ContractData changes containing
  `Symbol("name")` = "MyToken" produces deployment.metadata with that
  name.
- **Unit** (`state.rs`): `detect_tokens` propagates name/symbol from
  deployment metadata into `ExtractedToken`.
- **Integration**: synthetic ledger with Fungible deploy + ContractData
  changes → `tokens` row has `name` / `symbol` populated.

## Acceptance Criteria

- [ ] `extract_contract_deployments` populates `metadata.name` and
      `metadata.symbol` when standard ContractData keys are present.
- [ ] `detect_tokens` threads those values into `ExtractedToken`.
- [ ] `upsert_tokens_soroban` writes non-NULL name/symbol.
- [ ] Late-WASM path — either Option A implemented, or 0124 scope
      explicitly extended (not both).
- [ ] Unit + integration coverage as above.
- [ ] Does NOT regress SAC / classic token paths (name typically
      already populated from asset_code / metadata JSON).
