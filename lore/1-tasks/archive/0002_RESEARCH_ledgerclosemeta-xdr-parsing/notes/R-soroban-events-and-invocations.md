---
title: 'Soroban events, ScVal decode, and invocation tree (Rust)'
type: research
status: mature
spawned_from: null
spawns: []
tags: [rust, soroban, events, scval, invocation-tree]
links:
  - https://github.com/stellar/rs-stellar-xdr
history:
  - date: 2026-03-26
    status: mature
    who: stkrolikiewicz
    note: 'Rewritten as Rust-first with stellar_xdr::curr types and stellar-indexer patterns'
---

# Soroban events, ScVal decode, and invocation tree (Rust)

## Event Types (ContractEventType)

| Type         | Description                                 | Storage                                         |
| ------------ | ------------------------------------------- | ----------------------------------------------- |
| `Contract`   | Application-level events from contract code | Always stored                                   |
| `System`     | System events (token transfers, etc.)       | Always stored                                   |
| `Diagnostic` | Debug events (diagnostic mode only)         | Stored if `contract_id` present, skip otherwise |

From [stellar-indexer events/mod.rs](../sources/stellar-indexer-events-mod-rs.md):

```rust
pub fn skip_diagnostic_without_contract(ev: &ContractEvent) -> bool {
    ev.type_ == ContractEventType::Diagnostic && ev.contract_id.is_none()
}
```

## V3 vs V4 Event Extraction (CRITICAL)

Full implementation from [stellar-indexer](../sources/stellar-indexer-events-mod-rs.md):

```rust
use stellar_xdr::curr::{ContractEvent, ContractEventBody, TransactionMeta};

pub fn events_from_tx_meta(meta: &TransactionMeta) -> Vec<&ContractEvent> {
    let mut out = Vec::new();
    match meta {
        TransactionMeta::V3(v3) => {
            if let Some(ref soroban) = v3.soroban_meta {
                for ev in soroban.events.iter() {
                    out.push(ev);
                }
                // Diagnostic events (filtered)
                for de in soroban.diagnostic_events.iter() {
                    if !skip_diagnostic_without_contract(&de.event) {
                        out.push(&de.event);
                    }
                }
            }
        }
        TransactionMeta::V4(v4) => {
            // Top-level events (with stage info)
            for te in v4.events.iter() {
                out.push(&te.event);
            }
            // Per-operation events
            for op_meta in v4.operations.iter() {
                for ev in op_meta.events.iter() {
                    out.push(ev);
                }
            }
            // Diagnostic events
            for de in v4.diagnostic_events.iter() {
                if !skip_diagnostic_without_contract(&de.event) {
                    out.push(&de.event);
                }
            }
        }
        _ => {}
    }
    out
}
```

**V4 key differences (introduced Protocol 23, CAP-0067):**

- Top-level `v4.events` contains **fee charge/refund events** (`TransactionEvent` with `stage` field), NOT Soroban contract events
- Per-operation events in `OperationMetaV2` (`op_meta.events`) — **this is where contract events live in V4**
- `v4.diagnostic_events` at top-level for debug events
- `v4.soroban_meta` still present as `SorobanTransactionMetaV2` (return value, ext) but events moved out

## ContractEvent → DB Fields

```rust
let ContractEventBody::V0(v0) = &event.body;

// DB: soroban_events table
let event_type: &str = event.type_.name();                       // → event_type
let contract_id: Option<String> = event.contract_id
    .as_ref()
    .map(|c| c.to_string());                                     // → contract_id
let topics: Vec<serde_json::Value> = v0.topics.iter()
    .map(scval_to_typed_json_value)
    .collect();                                                   // → topics (JSONB)
let data: serde_json::Value = scval_to_typed_json_value(&v0.data); // → data (JSONB)
```

## ScVal Decode — Typed JSON Format

From [stellar-indexer scval/mod.rs](../sources/stellar-indexer-scval-mod-rs.md). The typed format `{ "type": "u128", "value": "123" }` preserves type info for JSONB:

```rust
use stellar_xdr::curr::ScVal;

pub fn scval_to_typed_json_value(v: &ScVal) -> serde_json::Value {
    let (type_name, value) = match v {
        ScVal::Bool(b)     => ("bool", json!(*b)),
        ScVal::Void        => ("void", json!(null)),
        ScVal::U32(x)      => ("u32", json!(*x)),
        ScVal::I32(x)      => ("i32", json!(*x)),
        ScVal::U64(x)      => ("u64", json!(*x)),
        ScVal::I64(x)      => ("i64", json!(*x)),
        ScVal::Timepoint(t) => ("timepoint", json!(u64::from(t.clone()))),
        ScVal::Duration(d) => ("duration", json!(d.0)),
        ScVal::U128(parts) => ("u128", json!(u128::from(parts).to_string())),
        ScVal::I128(parts) => ("i128", json!(i128::from(parts).to_string())),
        ScVal::U256(_) | ScVal::I256(_) => ("u256/i256", json!(format!("{:?}", v))),
        ScVal::Bytes(b)    => ("bytes", json!(base64_encode(b.0.as_slice()))),
        ScVal::String(s)   => ("string", json!(std::str::from_utf8(s.0.as_slice()).unwrap_or("<invalid-utf8>"))),
        ScVal::Symbol(s)   => ("sym", json!(std::str::from_utf8(s.0.as_slice()).unwrap_or("<invalid-utf8>"))),
        ScVal::Vec(Some(vec)) => ("vec", json!(vec.iter().map(scval_to_typed_json_value).collect::<Vec<_>>())),
        ScVal::Vec(None)   => ("vec", json!(null)),
        ScVal::Map(Some(m)) => ("map", json!(m.iter().map(|e| json!({
            "key": scval_to_typed_json_value(&e.key),
            "value": scval_to_typed_json_value(&e.val),
        })).collect::<Vec<_>>())),
        ScVal::Map(None)   => ("map", json!(null)),
        ScVal::Address(a)  => ("address", json!(a.to_string())),
        ScVal::ContractInstance(_) => ("contract_instance", json!("...")),
        ScVal::LedgerKeyContractInstance => ("ledger_key_contract_instance", json!(null)),
        ScVal::LedgerKeyNonce(k) => ("ledger_key_nonce", json!(format!("{:?}", k))),
        ScVal::Error(e)    => ("error", json!(format!("{:?}", e))),
    };
    json!({ "type": type_name, "value": value })
}
```

### ScVal Type Mapping: XDR → Rust → JSONB

| ScVal Type         | Rust Type         | JSON `type`           | JSON `value`                 |
| ------------------ | ----------------- | --------------------- | ---------------------------- |
| `Bool(b)`          | `bool`            | `"bool"`              | `true` / `false`             |
| `Void`             | —                 | `"void"`              | `null`                       |
| `U32(x)`           | `u32`             | `"u32"`               | `42`                         |
| `I32(x)`           | `i32`             | `"i32"`               | `-7`                         |
| `U64(x)`           | `u64`             | `"u64"`               | `123456789`                  |
| `I64(x)`           | `i64`             | `"i64"`               | `-42`                        |
| `U128(parts)`      | `u128`            | `"u128"`              | `"340282..."` (string)       |
| `I128(parts)`      | `i128`            | `"i128"`              | `"-170141..."` (string)      |
| `Timepoint(t)`     | `u64`             | `"timepoint"`         | `1774455216`                 |
| `Duration(d)`      | `u64`             | `"duration"`          | `3600`                       |
| `Bytes(b)`         | `&[u8]`           | `"bytes"`             | base64 string                |
| `String(s)`        | `&[u8]`           | `"string"`            | `"hello"`                    |
| `Symbol(s)`        | `&[u8]`           | `"sym"`               | `"transfer"`                 |
| `Address(a)`       | `ScAddress`       | `"address"`           | `"GABC..."` / `"CABC..."`    |
| `Vec(Some(v))`     | `Vec<ScVal>`      | `"vec"`               | recursive array              |
| `Map(Some(m))`     | `Vec<ScMapEntry>` | `"map"`               | `[{"key":..., "value":...}]` |
| `ContractInstance` | struct            | `"contract_instance"` | `"..."`                      |

**Why typed format:** GIN indexes on JSONB can query by `type` field. Frontend renders appropriately (i128 as formatted number vs string). No ambiguity between `"42"` (string) and `42` (number).

## Invocation Tree

The invocation hierarchy comes from `InvokeHostFunctionOp.auth`, NOT from `soroban_meta` or `result_meta_xdr`.

### Structure

```
SorobanAuthorizationEntry
├── credentials: SorobanCredentials
└── root_invocation: SorobanAuthorizedInvocation
    ├── function: SorobanAuthorizedFunction
    │   └── InvokeContract(InvokeContractArgs)
    │       ├── contract_address: ScAddress
    │       ├── function_name: ScSymbol
    │       └── args: Vec<ScVal>
    └── sub_invocations: Vec<SorobanAuthorizedInvocation>  ← recursive
```

### Extraction for `operation_tree` JSONB

```rust
use stellar_xdr::curr::{
    HostFunction, OperationBody, SorobanAuthorizedFunction, SorobanAuthorizedInvocation,
};

fn invocation_to_json(inv: &SorobanAuthorizedInvocation) -> serde_json::Value {
    let call = match &inv.function {
        SorobanAuthorizedFunction::SorobanAuthorizedFunctionTypeContractFn(args) => {
            json!({
                "type": "invokeContract",
                "contractId": args.contract_address.to_string(),
                "functionName": std::str::from_utf8(args.function_name.0.as_slice())
                    .unwrap_or("<non-utf8>"),
                "args": args.args.iter().map(scval_to_typed_json_value).collect::<Vec<_>>(),
            })
        }
        SorobanAuthorizedFunction::SorobanAuthorizedFunctionTypeCreateContractHostFn(args) => {
            json!({ "type": "createContract" })
        }
        SorobanAuthorizedFunction::SorobanAuthorizedFunctionTypeCreateContractV2HostFn(args) => {
            json!({ "type": "createContractV2" })
        }
    };

    json!({
        "call": call,
        "subInvocations": inv.sub_invocations.iter()
            .map(invocation_to_json)
            .collect::<Vec<_>>(),
    })
}

// Extract from operation
fn extract_invocation_tree(op: &Operation) -> Option<serde_json::Value> {
    if let OperationBody::InvokeHostFunction(ihf) = &op.body {
        let trees: Vec<_> = ihf.auth.iter().map(|entry| {
            json!({
                "credentials": entry.credentials.name(),
                "rootInvocation": invocation_to_json(&entry.root_invocation),
            })
        }).collect();
        if !trees.is_empty() { return Some(json!(trees)); }
    }
    None
}
```

### Soroban Resources from Envelope

From [stellar-indexer json_build/mod.rs](../sources/stellar-indexer-json-build-mod-rs.md):

```rust
use stellar_xdr::curr::TransactionExt;

fn soroban_resources(env: &TransactionEnvelope) -> Option<serde_json::Value> {
    let tx = match env {
        TransactionEnvelope::Tx(v1) => &v1.tx,
        TransactionEnvelope::TxFeeBump(fb) => {
            let FeeBumpTransactionInnerTx::Tx(inner) = &fb.tx.inner_tx;
            &inner.tx
        }
        _ => return None,
    };
    let TransactionExt::V1(soroban_data) = &tx.ext else { return None };
    Some(json!({
        "instructions": soroban_data.resources.instructions,
        "disk_read_bytes": soroban_data.resources.disk_read_bytes,
        "write_bytes": soroban_data.resources.write_bytes,
    }))
}
```

## Return Value

```rust
// V3
if let TransactionMeta::V3(v3) = meta {
    if let Some(ref soroban) = v3.soroban_meta {
        let rv = scval_to_typed_json_value(&soroban.return_value);
    }
}
// V4 — soroban_meta is SorobanTransactionMetaV2, may be None
if let TransactionMeta::V4(v4) = meta {
    if let Some(ref soroban) = v4.soroban_meta {
        let rv = scval_to_typed_json_value(&soroban.return_value);
    }
}
```

## SAC Event Signatures (CAP-0046-06 / SEP-0041)

Complete list of Stellar Asset Contract event topic symbols. Used for `event_interpretations` table population.

### Pre-CAP-0067 (Protocol < 23)

| Event            | First Topic (`Symbol`) | Additional Topics                      | Data                                     |
| ---------------- | ---------------------- | -------------------------------------- | ---------------------------------------- |
| `transfer`       | `"transfer"`           | `Address(from)`, `Address(to)`         | `I128(amount)`                           |
| `mint`           | `"mint"`               | `Address(admin)`, `Address(to)`        | `I128(amount)`                           |
| `burn`           | `"burn"`               | `Address(from)`                        | `I128(amount)`                           |
| `clawback`       | `"clawback"`           | `Address(admin)`, `Address(from)`      | `I128(amount)`                           |
| `approve`        | `"approve"`            | `Address(from)`, `Address(spender)`    | `I128(amount)`, `U32(live_until_ledger)` |
| `set_admin`      | `"set_admin"`          | `Address(admin)`, `Address(new_admin)` | —                                        |
| `set_authorized` | `"set_authorized"`     | `Address(admin)`, `Address(id)`        | `U32(authorize_flag)`                    |

### Post-CAP-0067 (Protocol 23+)

CAP-0067 removed admin topics from `mint`, `clawback`, and `set_authorized`. SAC now emits `mint`/`burn` instead of `transfer` when the issuer is involved. All SAC events now include an asset identifier topic (SEP-0011 format).

| Event            | First Topic (`Symbol`) | Additional Topics                                    | Data                                     |
| ---------------- | ---------------------- | ---------------------------------------------------- | ---------------------------------------- |
| `transfer`       | `"transfer"`           | `Address(from)`, `Address(to)`, `Symbol(asset)`      | `I128(amount)`                           |
| `mint`           | `"mint"`               | `Address(to)`, `Symbol(asset)`                       | `I128(amount)`                           |
| `burn`           | `"burn"`               | `Address(from)`, `Symbol(asset)`                     | `I128(amount)`                           |
| `clawback`       | `"clawback"`           | `Address(from)`, `Symbol(asset)`                     | `I128(amount)`                           |
| `approve`        | `"approve"`            | `Address(from)`, `Address(spender)`, `Symbol(asset)` | `I128(amount)`, `U32(live_until_ledger)` |
| `set_admin`      | `"set_admin"`          | `Address(admin)`                                     | `Address(new_admin)`                     |
| `set_authorized` | `"set_authorized"`     | `Address(id)`                                        | `U32(authorize_flag)`                    |

**Note:** Implementation must handle both pre- and post-CAP-0067 formats for historical data. Detect protocol version from the enclosing `LedgerCloseMeta`'s ledger header (for example `meta.v4.ledger_header.header.ledger_version`) to determine which format applies.
