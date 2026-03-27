---
title: 'Field-by-field extraction mapping: XDR Rust path to database column'
type: research
status: mature
spawned_from: null
spawns: []
tags: [rust, field-mapping, database, xdr]
links: []
history:
  - date: 2026-03-26
    status: mature
    who: stkrolikiewicz
    note: 'Rewritten as Rust-first with stellar_xdr::curr struct field paths'
---

# Field-by-field extraction mapping: XDR Rust path to database column

## Conventions

- `v` = `LedgerCloseMetaV2` (from `LedgerCloseMeta::V2(v)`)
- `env` = `&TransactionEnvelope` (extracted via `for_each_envelope()`)
- `proc` = `&v.tx_processing[i]` (`TransactionResultMeta`)
- `meta` = `&proc.tx_apply_processing` (`TransactionMeta` — match on V3/V4)
- All paths are Rust struct field access (`.field`)

## Coverage

This mapping covers all **12 tables** from the [database schema overview](../../../../docs/architecture/database-schema/database-schema-overview.md). Tables 1–6, 9, and 10 have their rows sourced directly from LedgerCloseMeta XDR fields (including `LedgerEntryChanges`), though individual columns in those tables may still be computed or enriched. Tables 7, 8, 11, and 12 are derived/enrichment tables whose rows are populated from combinations of XDR data and event interpretation logic.

## Table 1: ledgers

| Column              | Rust Path                                                        |
| ------------------- | ---------------------------------------------------------------- |
| `sequence`          | `v.ledger_header.header.ledger_seq`                              |
| `hash`              | `hex::encode(Sha256::digest(&v.ledger_header.to_xdr(limits)?))`  |
| `closed_at`         | `u64::from(v.ledger_header.header.scp_value.close_time.clone())` |
| `protocol_version`  | `v.ledger_header.header.ledger_version`                          |
| `transaction_count` | `v.tx_processing.len()`                                          |
| `base_fee`          | `v.ledger_header.header.base_fee`                                |

## Table 2: transactions

| Column            | Rust Path                                                                       |
| ----------------- | ------------------------------------------------------------------------------- |
| `hash`            | `hex::encode(proc.result.transaction_hash.0)` or `env.hash(network_id)?`        |
| `ledger_sequence` | (from parent ledger)                                                            |
| `source_account`  | `envelope_sender_and_ops(env).0` (see R-sdk-types)                              |
| `fee_charged`     | `proc.result.result.fee_charged`                                                |
| `successful`      | `matches!(proc.result.result.result, TxSuccess(_) \| TxFeeBumpInnerSuccess(_))` |
| `result_code`     | `proc.result.result.result.name()`                                              |
| `envelope_xdr`    | `base64_encode(env.to_xdr(limits)?)`                                            |
| `result_xdr`      | `base64_encode(proc.result.result.to_xdr(limits)?)`                             |
| `result_meta_xdr` | `base64_encode(proc.tx_apply_processing.to_xdr(limits)?)`                       |
| `memo_type`       | `match &inner_tx.memo { Memo::None => "none", ... }`                            |
| `memo`            | memo value extraction per type                                                  |
| `created_at`      | Same as ledger `closed_at`: `v.ledger_header.header.scp_value.close_time`       |
| `parse_error`     | Set to `true` when inner parsing fails (see R-error-handling)                   |
| `operation_tree`  | `extract_invocation_tree(op)` (see R-soroban-events)                            |

## Table 3: operations

| Column           | Rust Path                                       |
| ---------------- | ----------------------------------------------- |
| `transaction_id` | FK to transactions                              |
| `type`           | `op.body.name()` (e.g., `"InvokeHostFunction"`) |
| `details`        | type-specific JSONB (see below)                 |

### Operation details JSONB — `InvokeHostFunction`

```rust
if let OperationBody::InvokeHostFunction(ihf) = &op.body {
    if let HostFunction::InvokeContract(args) = &ihf.host_function {
        json!({
            "contractId": args.contract_address.to_string(),
            "functionName": std::str::from_utf8(args.function_name.0.as_slice()).unwrap_or("<non-utf8>"),
            "functionArgs": args.args.iter().map(scval_to_typed_json_value).collect::<Vec<_>>(),
            "hostFunctionType": "InvokeContract",
        })
    }
}
```

### Operation details JSONB — `Payment`

```rust
if let OperationBody::Payment(pay) = &op.body {
    json!({
        "destination": pay.destination.to_string(),
        "amount": pay.amount,
        "asset": format_asset(&pay.asset),
    })
}
```

## Table 4: soroban_contracts

Populated from `LedgerEntryChanges` where entry is `ContractData` with key `LedgerKeyContractInstance`.

| Column               | Rust Path                                                               |
| -------------------- | ----------------------------------------------------------------------- |
| `contract_id`        | `cd.contract.to_string()` (if `ScAddress::Contract`)                    |
| `wasm_hash`          | `ContractExecutable::Wasm(hash)` from contract instance val             |
| `deployer_account`   | TX source account                                                       |
| `deployed_at_ledger` | ledger sequence                                                         |
| `contract_type`      | Derived: classify from known interface patterns or event signatures     |
| `is_sac`             | `matches!(executable, ContractExecutable::StellarAsset)` — true for SAC |
| `metadata`           | JSONB: contract name, interface ABI if available from ContractInstance  |
| `search_vector`      | Generated column (PostgreSQL) — not populated from XDR directly         |

Detection:

```rust
if let LedgerEntryData::ContractData(cd) = &entry.data {
    if cd.key == ScVal::LedgerKeyContractInstance {
        let contract_id = cd.contract.to_string();
        // Extract executable type
        if let ScVal::ContractInstance(inst) = &cd.val {
            let is_sac = matches!(inst.executable, ContractExecutable::StellarAsset);
            let wasm_hash = match &inst.executable {
                ContractExecutable::Wasm(hash) => Some(hex::encode(hash.0)),
                ContractExecutable::StellarAsset => None,
            };
        }
    }
}
```

## Table 5: soroban_invocations

| Column            | Rust Path                                                       |
| ----------------- | --------------------------------------------------------------- |
| `contract_id`     | `args.contract_address.to_string()`                             |
| `caller_account`  | TX source account                                               |
| `function_name`   | `std::str::from_utf8(args.function_name.0.as_slice())`          |
| `function_args`   | `args.args.iter().map(scval_to_typed_json_value)` (JSONB)       |
| `return_value`    | `scval_to_typed_json_value(&soroban_meta.return_value)` (JSONB) |
| `successful`      | from TX result                                                  |
| `ledger_sequence` | from parent ledger `v.ledger_header.header.ledger_seq`          |
| `created_at`      | Same as ledger `closed_at`                                      |

## Table 6: soroban_events

| Column            | Rust Path                                                 |
| ----------------- | --------------------------------------------------------- |
| `contract_id`     | `event.contract_id.as_ref().map(\|c\| c.to_string())`     |
| `event_type`      | `event.type_.name()`                                      |
| `topics`          | `v0.topics.iter().map(scval_to_typed_json_value)` (JSONB) |
| `data`            | `scval_to_typed_json_value(&v0.data)` (JSONB)             |
| `ledger_sequence` | from parent ledger `v.ledger_header.header.ledger_seq`    |
| `created_at`      | Same as ledger `closed_at`                                |

V3: `meta.v3().soroban_meta.events`
V4: fee/system events: `meta.v4().events` (top-level); per-op contract events: `meta.v4().operations[i].events`

## Table 9: accounts

From `LedgerEntryChanges` where entry type is `Account`:

| Column              | Rust Path                                             |
| ------------------- | ----------------------------------------------------- |
| `account_id`        | `entry.account_id.to_string()`                        |
| `sequence_number`   | `entry.seq_num.0`                                     |
| `balances`          | `entry.balance` (native) + trustlines (JSONB)         |
| `home_domain`       | `std::str::from_utf8(entry.home_domain.0.as_slice())` |
| `first_seen_ledger` | Ledger sequence on first `Created` change             |
| `last_seen_ledger`  | Ledger sequence on most recent `Created`/`Updated`    |

`AccountEntry` fields: `account_id, balance, seq_num, num_sub_entries, inflation_dest, flags, home_domain, thresholds, signers, ext`

**Balances JSONB construction:** Native balance from `AccountEntry.balance`. Trustline balances aggregated from `LedgerEntryData::Trustline(tl)` entries where `tl.account_id` matches, structured as:

```rust
json!([
    { "asset": "native", "balance": entry.balance },
    { "asset": format_asset(&tl.asset), "balance": tl.balance, "limit": tl.limit }
])
```

## Table 10: liquidity_pools

From `LedgerEntryChanges` where entry type is `LiquidityPool`:

| Column                | Rust Path                                                 |
| --------------------- | --------------------------------------------------------- |
| `pool_id`             | `hex::encode(entry.liquidity_pool_id.0)`                  |
| `asset_a`             | `format_asset(&cp.params.asset_a)` (JSONB)                |
| `asset_b`             | `format_asset(&cp.params.asset_b)` (JSONB)                |
| `fee_bps`             | `cp.params.fee`                                           |
| `reserves`            | `json!({ "a": cp.reserve_a, "b": cp.reserve_b })` (JSONB) |
| `total_shares`        | `cp.total_pool_shares`                                    |
| `tvl`                 | Derived: computed from reserves × asset prices (not XDR)  |
| `created_at_ledger`   | Ledger sequence on first `Created` change                 |
| `last_updated_ledger` | Ledger sequence on most recent `Created`/`Updated`        |

```rust
if let LedgerEntryData::LiquidityPool(lp) = &entry.data {
    let LiquidityPoolEntryBody::ConstantProduct(cp) = &lp.body;
    // cp.params.asset_a, cp.params.asset_b, cp.params.fee
    // cp.reserve_a, cp.reserve_b, cp.total_pool_shares
}
```

## LedgerEntryChanges Iteration

```rust
// Changes come from 3 locations in TransactionMetaV3/V4:
let changes_before = &v34.tx_changes_before;  // pre-tx state
let changes_after = &v34.tx_changes_after;    // post-tx state
// Per-operation:
for op_meta in v34.operations.iter() {
    for change in op_meta.changes.0.iter() {
        match change {
            LedgerEntryChange::Created(entry) => {
                match &entry.data {
                    LedgerEntryData::Account(acct) => { /* accounts table */ }
                    LedgerEntryData::Trustline(tl) => { /* trustline balances */ }
                    LedgerEntryData::ContractData(cd) => { /* soroban_contracts */ }
                    LedgerEntryData::ContractCode(cc) => { /* WASM code */ }
                    LedgerEntryData::LiquidityPool(lp) => { /* liquidity_pools */ }
                    LedgerEntryData::Offer(o) => { /* DEX offers */ }
                    LedgerEntryData::Ttl(ttl) => { /* TTL extensions */ }
                    _ => {}
                }
            }
            LedgerEntryChange::Updated(entry) => { /* same match */ }
            LedgerEntryChange::State(entry) => { /* pre-change state snapshot */ }
            LedgerEntryChange::Removed(key) => { /* LedgerKey, not full entry */ }
        }
    }
}
```

## LedgerEntry Types (10 total)

| Type               | Rust Enum                                            | Key Fields                                                |
| ------------------ | ---------------------------------------------------- | --------------------------------------------------------- |
| `Account`          | `LedgerEntryData::Account(AccountEntry)`             | account_id, balance, seq_num, home_domain, flags, signers |
| `Trustline`        | `LedgerEntryData::Trustline(TrustLineEntry)`         | account_id, asset, balance, limit, flags                  |
| `Offer`            | `LedgerEntryData::Offer(OfferEntry)`                 | DEX order book entries                                    |
| `Data`             | `LedgerEntryData::Data(DataEntry)`                   | account key-value data                                    |
| `ClaimableBalance` | `LedgerEntryData::ClaimableBalance(...)`             | pending claims                                            |
| `LiquidityPool`    | `LedgerEntryData::LiquidityPool(LiquidityPoolEntry)` | pool_id, body (ConstantProduct)                           |
| `ContractData`     | `LedgerEntryData::ContractData(ContractDataEntry)`   | contract, key, durability, val                            |
| `ContractCode`     | `LedgerEntryData::ContractCode(ContractCodeEntry)`   | hash, code (WASM bytes)                                   |
| `ConfigSetting`    | `LedgerEntryData::ConfigSetting(...)`                | network config                                            |
| `Ttl`              | `LedgerEntryData::Ttl(TtlEntry)`                     | key_hash, live_until_ledger_seq                           |

---

## Derived / Enrichment Tables

These tables are not populated directly from a single XDR field path. They are computed from combinations of XDR data, event pattern matching, and enrichment logic.

## Table 7: event_interpretations

Enrichment layer populated **after** soroban_events are stored. Not directly from XDR — derived from event topic/data pattern matching.

| Column                | Source                                                                          |
| --------------------- | ------------------------------------------------------------------------------- |
| `event_id`            | FK to `soroban_events.id`                                                       |
| `interpretation_type` | Pattern match on `topics[0]` symbol: `"transfer"`, `"mint"`, `"burn"`, `"swap"` |
| `human_readable`      | Constructed string, e.g., `"Transfer 100 USDC from GABC… to GDEF…"`             |
| `structured_data`     | JSONB with decoded fields per type (see below)                                  |

**XDR source chain:** `ContractEvent.body.v0.topics` → match known SAC/token patterns:

```rust
// SAC transfer event: topics = [Symbol("transfer"), Address(from), Address(to), Symbol(asset_name)]
// SAC mint event:     topics = [Symbol("mint"), Address(admin), Address(to)]
// SAC burn event:     topics = [Symbol("burn"), Address(from)]
let topic_sym = match &v0.topics[0] {
    ScVal::Symbol(s) => std::str::from_utf8(s.0.as_slice()).unwrap_or(""),
    _ => "",
};
match topic_sym {
    "transfer" => interpret_transfer(&v0.topics, &v0.data),
    "mint" => interpret_mint(&v0.topics, &v0.data),
    "burn" => interpret_burn(&v0.topics, &v0.data),
    _ => None, // Unknown pattern — no interpretation
}
```

**Note:** Interpretation patterns are defined by CAP-0046-06 (Stellar Asset Contract). Custom contract events may follow the same convention but are not guaranteed.

## Table 8: tokens

Unified token model. Populated from multiple XDR sources depending on `asset_type`:

| Column           | Source                                                         |
| ---------------- | -------------------------------------------------------------- |
| `asset_type`     | `"classic"` / `"sac"` / `"soroban"` — determined by entry type |
| `asset_code`     | Classic: `Asset::CreditAlphanum4/12` code field                |
| `issuer_address` | Classic: `Asset.issuer.to_string()`                            |
| `contract_id`    | Soroban/SAC: from `ContractDataEntry.contract.to_string()`     |
| `name`           | From contract metadata or token `name()` invocation result     |
| `total_supply`   | From `mint`/`burn` event aggregation or contract state         |
| `holder_count`   | Derived: count of unique addresses from transfer events        |
| `metadata`       | JSONB: symbol, decimals, etc. from token interface calls       |

**XDR sources by asset_type:**

- **`classic`**: From `LedgerEntryData::Trustline(tl)` — `tl.asset` contains `Asset::CreditAlphanum4 { asset_code, issuer }` or `Asset::CreditAlphanum12 { asset_code, issuer }`. First trustline `Created` event for a new asset creates the token record.
- **`sac`**: From `LedgerEntryData::ContractData(cd)` where `ContractExecutable::StellarAsset`. The SAC wraps a classic asset — link via `asset_code + issuer` to the classic token. See CAP-0046-06.
- **`soroban`**: From `LedgerEntryData::ContractData(cd)` where contract implements token interface (detected via `function_name` in invocations: `name`, `symbol`, `decimals`, `balance`, `transfer`).

## Table 11: nfts

Populated from event interpretation of NFT contract patterns.

| Column             | Source                                                                 |
| ------------------ | ---------------------------------------------------------------------- |
| `contract_id`      | From `ContractEvent.contract_id` on mint/transfer events               |
| `token_id`         | From event `data` or `topics` — ScVal decoded, contract-specific       |
| `collection_name`  | From contract metadata JSONB (if available)                            |
| `owner_account`    | From most recent `transfer` event `to` address                         |
| `name`             | From contract metadata or token-specific query                         |
| `media_url`        | From contract metadata JSONB (if available)                            |
| `metadata`         | JSONB: all available NFT attributes                                    |
| `minted_at_ledger` | Ledger sequence of first `mint` event for this (contract_id, token_id) |
| `last_seen_ledger` | Ledger sequence of most recent transfer/update event                   |

**XDR source chain:** NFT contracts have no standardized Stellar interface yet. Detection relies on heuristic event patterns — typically `mint(to, token_id, ...)` and `transfer(from, to, token_id, ...)` where `token_id` is a unique ScVal (usually `U128` or `String`).

**Note:** NFT support is best-effort. Unlike SAC tokens (CAP-0046-06), there is no Stellar CAP for NFTs. Each NFT contract may use different event signatures. Implementation should support configurable pattern matchers.

## Table 12: liquidity_pool_snapshots

Append-only time-series captures of LP state. Fields mirror `liquidity_pools` at a point in time.

| Column            | Source                                              |
| ----------------- | --------------------------------------------------- |
| `pool_id`         | FK to `liquidity_pools.pool_id`                     |
| `ledger_sequence` | from parent ledger                                  |
| `created_at`      | Same as ledger `closed_at`                          |
| `reserves`        | Same as `liquidity_pools.reserves` at snapshot time |
| `total_shares`    | Same as `liquidity_pools.total_shares`              |
| `tvl`             | Derived: reserves × asset prices                    |
| `volume`          | Derived: sum of swap amounts in period              |
| `fee_revenue`     | Derived: sum of fees collected in period            |

**XDR source:** Same as `liquidity_pools` — `LedgerEntryData::LiquidityPool(lp)`. Snapshot created on every `Updated` change to a pool entry. `volume` and `fee_revenue` are computed from associated `swap` events/operations in the same ledger.

---

## Mainnet Distribution (Ledger 61827432, 343 txs)

| Entry Type    | Count | Target Table             |
| ------------- | ----- | ------------------------ |
| Trustline     | 1,365 | token balances           |
| ContractData  | 1,056 | soroban_contracts, state |
| Ttl           | 490   | TTL extensions           |
| Account       | 217   | accounts                 |
| Offer         | 215   | DEX orderbook            |
| LiquidityPool | 56    | liquidity_pools          |
