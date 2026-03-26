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

## Table: ledgers

| Column              | Rust Path                                                        |
| ------------------- | ---------------------------------------------------------------- |
| `sequence`          | `v.ledger_header.header.ledger_seq`                              |
| `hash`              | `hex::encode(Sha256::digest(&v.ledger_header.to_xdr(limits)?))`  |
| `closed_at`         | `u64::from(v.ledger_header.header.scp_value.close_time.clone())` |
| `protocol_version`  | `v.ledger_header.header.ledger_version`                          |
| `transaction_count` | `v.tx_processing.len()`                                          |
| `base_fee`          | `v.ledger_header.header.base_fee`                                |

## Table: transactions

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
| `operation_tree`  | `extract_invocation_tree(op)` (see R-soroban-events)                            |

## Table: operations

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

## Table: soroban_contracts

Populated from `LedgerEntryChanges` where entry is `ContractData` with key `LedgerKeyContractInstance`.

| Column               | Rust Path                                                               |
| -------------------- | ----------------------------------------------------------------------- |
| `contract_id`        | `entry.contract_data().contract.to_string()` (if `ScAddress::Contract`) |
| `wasm_hash`          | `ContractExecutable::Wasm(hash)` from contract instance val             |
| `deployer_account`   | TX source account                                                       |
| `deployed_at_ledger` | ledger sequence                                                         |

Detection:

```rust
if let LedgerEntryData::ContractData(cd) = &entry.data {
    if cd.key == ScVal::LedgerKeyContractInstance {
        // This is a contract instance entry
        let contract_id = cd.contract.to_string();
    }
}
```

## Table: soroban_invocations

| Column           | Rust Path                                                       |
| ---------------- | --------------------------------------------------------------- |
| `contract_id`    | `args.contract_address.to_string()`                             |
| `caller_account` | TX source account                                               |
| `function_name`  | `std::str::from_utf8(args.function_name.0.as_slice())`          |
| `function_args`  | `args.args.iter().map(scval_to_typed_json_value)` (JSONB)       |
| `return_value`   | `scval_to_typed_json_value(&soroban_meta.return_value)` (JSONB) |
| `successful`     | from TX result                                                  |

## Table: soroban_events

| Column        | Rust Path                                                 |
| ------------- | --------------------------------------------------------- |
| `contract_id` | `event.contract_id.as_ref().map(\|c\| c.to_string())`     |
| `event_type`  | `event.type_.name()`                                      |
| `topics`      | `v0.topics.iter().map(scval_to_typed_json_value)` (JSONB) |
| `data`        | `scval_to_typed_json_value(&v0.data)` (JSONB)             |

V3: `meta.v3().soroban_meta.events`
V4: `meta.v4().events` (top-level) + `meta.v4().operations[i].events` (per-op)

## Table: accounts

From `LedgerEntryChanges` where entry type is `Account`:

| Column            | Rust Path                                             |
| ----------------- | ----------------------------------------------------- |
| `account_id`      | `entry.account_id.to_string()`                        |
| `sequence_number` | `entry.seq_num.0`                                     |
| `balances`        | `entry.balance` (native) + trustlines (JSONB)         |
| `home_domain`     | `std::str::from_utf8(entry.home_domain.0.as_slice())` |

`AccountEntry` fields: `account_id, balance, seq_num, num_sub_entries, inflation_dest, flags, home_domain, thresholds, signers, ext`

## Table: liquidity_pools

From `LedgerEntryChanges` where entry type is `LiquidityPool`:

| Column         | Rust Path                                      |
| -------------- | ---------------------------------------------- |
| `pool_id`      | `hex::encode(entry.liquidity_pool_id.0)`       |
| `asset_a`      | `cp.params.asset_a` (JSONB)                    |
| `asset_b`      | `cp.params.asset_b` (JSONB)                    |
| `fee_bps`      | `cp.params.fee`                                |
| `reserves`     | `{ a: cp.reserve_a, b: cp.reserve_b }` (JSONB) |
| `total_shares` | `cp.total_pool_shares`                         |

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

## Mainnet Distribution (Ledger 61827432, 343 txs)

| Entry Type    | Count | Target Table             |
| ------------- | ----- | ------------------------ |
| Trustline     | 1,365 | token balances           |
| ContractData  | 1,056 | soroban_contracts, state |
| Ttl           | 490   | TTL extensions           |
| Account       | 217   | accounts                 |
| Offer         | 215   | DEX orderbook            |
| LiquidityPool | 56    | liquidity_pools          |
