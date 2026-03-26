---
source: rumblefishdev/stellar-indexer (private repo)
file: src/scval/mod.rs
retrieved: 2026-03-26
note: Copied from local checkout. Complete ScVal-to-JSON conversion with typed format.
---

# stellar-indexer: src/scval/mod.rs

ScVal conversion to JSON. Three functions:

- `format_scval` — human-readable string
- `scval_to_json_value` — native JSON (maps become objects keyed by symbol/string)
- `scval_to_typed_json_value` — typed format: `{ "type": "u128", "value": "123" }`

The typed format is the recommended approach for JSONB storage — preserves type info for frontend rendering and GIN indexing.

```rust
use serde_json::json;
use stellar_xdr::curr::{MuxedAccount, ScVal};

pub fn scval_to_typed_json_value(v: &ScVal) -> serde_json::Value {
    use base64::Engine;
    let (type_name, value) = match v {
        ScVal::Bool(b) => ("bool", json!(*b)),
        ScVal::Void => ("void", json!(null)),
        ScVal::Error(e) => ("error", json!(format!("{:?}", e))),
        ScVal::U32(x) => ("u32", json!(*x)),
        ScVal::I32(x) => ("i32", json!(*x)),
        ScVal::U64(x) => ("u64", json!(*x)),
        ScVal::I64(x) => ("i64", json!(*x)),
        ScVal::Timepoint(t) => {
            let u: u64 = t.clone().into();
            ("timepoint", json!(u))
        }
        ScVal::Duration(d) => ("duration", json!(d.0)),
        ScVal::U128(parts) => ("u128", json!(u128::from(parts).to_string())),
        ScVal::I128(parts) => ("i128", json!(i128::from(parts).to_string())),
        ScVal::U256(_) | ScVal::I256(_) => ("u256/i256", json!(format!("{:?}", v))),
        ScVal::Bytes(b) => (
            "bytes",
            json!(base64::engine::general_purpose::STANDARD.encode(b.0.as_slice())),
        ),
        ScVal::String(s) => (
            "string",
            json!(std::str::from_utf8(s.0.as_slice()).unwrap_or("<invalid-utf8>")),
        ),
        ScVal::Symbol(s) => (
            "sym",
            json!(std::str::from_utf8(s.0.as_slice()).unwrap_or("<invalid-utf8>")),
        ),
        ScVal::Vec(Some(vec)) => (
            "vec",
            json!(vec
                .iter()
                .map(scval_to_typed_json_value)
                .collect::<Vec<_>>()),
        ),
        ScVal::Vec(None) => ("vec", json!(null)),
        ScVal::Map(Some(m)) => (
            "map",
            json!(m.iter()
                .map(|e| json!({
                    "key": scval_to_typed_json_value(&e.key),
                    "value": scval_to_typed_json_value(&e.val),
                }))
                .collect::<Vec<_>>()),
        ),
        ScVal::Map(None) => ("map", json!(null)),
        ScVal::Address(a) => ("address", json!(a.to_string())),
        ScVal::ContractInstance(_) => ("contract_instance", json!("...")),
        ScVal::LedgerKeyContractInstance => ("ledger_key_contract_instance", json!(null)),
        ScVal::LedgerKeyNonce(k) => ("ledger_key_nonce", json!(format!("{:?}", k))),
    };
    json!({ "type": type_name, "value": value })
}
```
