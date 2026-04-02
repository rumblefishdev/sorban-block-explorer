//! ScVal to typed JSON decoder.
//!
//! Converts Stellar `ScVal` values into tagged JSON representations
//! for JSONB storage in PostgreSQL. Used by operations (0025),
//! events/invocations (0026), and entry changes (0027).
//!
//! Output format: `{ "type": "<type_name>", "value": <json_value> }`
//! This tagged format allows consumers to distinguish types unambiguously.

use base64::Engine;
use serde_json::{Value, json};
use stellar_xdr::curr::ScVal;

/// Decode an `ScVal` into a tagged JSON value: `{ "type": "...", "value": ... }`.
pub fn scval_to_typed_json(v: &ScVal) -> Value {
    let (type_name, value) = match v {
        ScVal::Bool(b) => ("bool", json!(*b)),
        ScVal::Void => ("void", json!(null)),
        ScVal::Error(e) => ("error", json!(e.name())),
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
        ScVal::U256(parts) => (
            "u256",
            json!(format!(
                "{:016x}{:016x}{:016x}{:016x}",
                parts.hi_hi, parts.hi_lo, parts.lo_hi, parts.lo_lo
            )),
        ),
        ScVal::I256(parts) => (
            "i256",
            json!(format!(
                "{:016x}{:016x}{:016x}{:016x}",
                parts.hi_hi, parts.hi_lo, parts.lo_hi, parts.lo_lo
            )),
        ),
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
            json!(vec.iter().map(scval_to_typed_json).collect::<Vec<_>>()),
        ),
        ScVal::Vec(None) => ("vec", json!(null)),
        ScVal::Map(Some(m)) => (
            "map",
            json!(
                m.iter()
                    .map(|e| json!({
                        "key": scval_to_typed_json(&e.key),
                        "value": scval_to_typed_json(&e.val),
                    }))
                    .collect::<Vec<_>>()
            ),
        ),
        ScVal::Map(None) => ("map", json!(null)),
        ScVal::Address(a) => ("address", json!(a.to_string())),
        ScVal::ContractInstance(inst) => {
            let executable = match &inst.executable {
                stellar_xdr::curr::ContractExecutable::Wasm(hash) => {
                    json!({ "type": "wasm", "hash": hex::encode(hash.0) })
                }
                stellar_xdr::curr::ContractExecutable::StellarAsset => {
                    json!({ "type": "stellar_asset" })
                }
            };
            ("contract_instance", json!({ "executable": executable }))
        }
        ScVal::LedgerKeyContractInstance => ("ledger_key_contract_instance", json!(null)),
        ScVal::LedgerKeyNonce(k) => ("ledger_key_nonce", json!(k.nonce)),
    };
    json!({ "type": type_name, "value": value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::*;

    #[test]
    fn bool_value() {
        let v = scval_to_typed_json(&ScVal::Bool(true));
        assert_eq!(v["type"], "bool");
        assert_eq!(v["value"], true);
    }

    #[test]
    fn void_value() {
        let v = scval_to_typed_json(&ScVal::Void);
        assert_eq!(v["type"], "void");
        assert!(v["value"].is_null());
    }

    #[test]
    fn u32_value() {
        let v = scval_to_typed_json(&ScVal::U32(42));
        assert_eq!(v["type"], "u32");
        assert_eq!(v["value"], 42);
    }

    #[test]
    fn i64_value() {
        let v = scval_to_typed_json(&ScVal::I64(-100));
        assert_eq!(v["type"], "i64");
        assert_eq!(v["value"], -100);
    }

    #[test]
    fn u128_as_string() {
        let parts = UInt128Parts { hi: 0, lo: 999 };
        let v = scval_to_typed_json(&ScVal::U128(parts));
        assert_eq!(v["type"], "u128");
        assert_eq!(v["value"], "999");
    }

    #[test]
    fn string_value() {
        let s = ScString::try_from("hello".as_bytes().to_vec()).unwrap();
        let v = scval_to_typed_json(&ScVal::String(s));
        assert_eq!(v["type"], "string");
        assert_eq!(v["value"], "hello");
    }

    #[test]
    fn symbol_value() {
        let s = ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap();
        let v = scval_to_typed_json(&ScVal::Symbol(s));
        assert_eq!(v["type"], "sym");
        assert_eq!(v["value"], "transfer");
    }

    #[test]
    fn nested_vec() {
        let inner = ScVal::U32(1);
        let vec = ScVec::try_from(vec![inner]).unwrap();
        let v = scval_to_typed_json(&ScVal::Vec(Some(vec)));
        assert_eq!(v["type"], "vec");
        assert_eq!(v["value"][0]["type"], "u32");
        assert_eq!(v["value"][0]["value"], 1);
    }

    #[test]
    fn map_value() {
        let entry = ScMapEntry {
            key: ScVal::Symbol(ScSymbol::try_from("k".as_bytes().to_vec()).unwrap()),
            val: ScVal::U32(7),
        };
        let map = ScMap::try_from(vec![entry]).unwrap();
        let v = scval_to_typed_json(&ScVal::Map(Some(map)));
        assert_eq!(v["type"], "map");
        assert_eq!(v["value"][0]["key"]["value"], "k");
        assert_eq!(v["value"][0]["value"]["value"], 7);
    }

    #[test]
    fn address_value() {
        let addr = ScAddress::Contract(ContractId(Hash([0xAB; 32])));
        let v = scval_to_typed_json(&ScVal::Address(addr));
        assert_eq!(v["type"], "address");
        assert!(v["value"].as_str().unwrap().len() > 0);
    }

    #[test]
    fn error_value() {
        let e = ScError::Budget(ScErrorCode::ExceededLimit);
        let v = scval_to_typed_json(&ScVal::Error(e));
        assert_eq!(v["type"], "error");
        assert!(v["value"].as_str().unwrap().len() > 0);
    }

    #[test]
    fn vec_none() {
        let v = scval_to_typed_json(&ScVal::Vec(None));
        assert_eq!(v["type"], "vec");
        assert!(v["value"].is_null());
    }

    #[test]
    fn map_none() {
        let v = scval_to_typed_json(&ScVal::Map(None));
        assert_eq!(v["type"], "map");
        assert!(v["value"].is_null());
    }

    #[test]
    fn duration_value() {
        let v = scval_to_typed_json(&ScVal::Duration(Duration(12345)));
        assert_eq!(v["type"], "duration");
        assert_eq!(v["value"], 12345);
    }

    #[test]
    fn timepoint_value() {
        let v = scval_to_typed_json(&ScVal::Timepoint(TimePoint(1700000000)));
        assert_eq!(v["type"], "timepoint");
        assert_eq!(v["value"], 1700000000u64);
    }

    #[test]
    fn contract_instance_wasm() {
        let inst = ScContractInstance {
            executable: ContractExecutable::Wasm(Hash([0xAA; 32])),
            storage: None,
        };
        let v = scval_to_typed_json(&ScVal::ContractInstance(inst));
        assert_eq!(v["type"], "contract_instance");
        assert_eq!(v["value"]["executable"]["type"], "wasm");
        assert_eq!(v["value"]["executable"]["hash"], "aa".repeat(32));
    }

    #[test]
    fn contract_instance_sac() {
        let inst = ScContractInstance {
            executable: ContractExecutable::StellarAsset,
            storage: None,
        };
        let v = scval_to_typed_json(&ScVal::ContractInstance(inst));
        assert_eq!(v["type"], "contract_instance");
        assert_eq!(v["value"]["executable"]["type"], "stellar_asset");
    }

    #[test]
    fn ledger_key_nonce() {
        let v = scval_to_typed_json(&ScVal::LedgerKeyNonce(ScNonceKey { nonce: 42 }));
        assert_eq!(v["type"], "ledger_key_nonce");
        assert_eq!(v["value"], 42);
    }

    #[test]
    fn bytes_base64() {
        let bytes = ScBytes::try_from(vec![0xDE, 0xAD]).unwrap();
        let v = scval_to_typed_json(&ScVal::Bytes(bytes));
        assert_eq!(v["type"], "bytes");
        assert_eq!(v["value"], "3q0=");
    }
}
