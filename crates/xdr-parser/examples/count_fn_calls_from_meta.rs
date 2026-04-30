//! Reads a base64-encoded `TransactionMeta` (the `result_meta_xdr`
//! field from Horizon) on stdin and prints a fn_call count summary as
//! JSON. Used to cross-check the indexer DB against Horizon mirrors
//! that retain meta (e.g. Lobstr's `horizon.stellar.lobstr.co`).
//!
//! Usage:
//!   curl -s https://horizon.stellar.lobstr.co/transactions/<HASH> \
//!     | jq -r .result_meta_xdr \
//!     | cargo run -p xdr-parser --example count_fn_calls_from_meta
//!
//! Counter walks `diagnostic_events` by hand and ignores
//! `extract_invocations_from_diagnostics` — independent code path,
//! not the parser under audit.

use base64::Engine;
use std::collections::BTreeMap;
use std::io::Read;

use stellar_xdr::curr::{
    ContractEventBody, ContractEventType, ContractId, Hash, Limits, ReadXdr, ScAddress, ScVal,
    TransactionMeta,
};

fn main() {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("read stdin");
    let b64 = input.trim();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("base64 decode");
    let meta = TransactionMeta::from_xdr(&bytes, Limits::none()).expect("xdr decode");

    let diags: Vec<_> = match &meta {
        TransactionMeta::V3(v3) => v3
            .soroban_meta
            .as_ref()
            .map(|m| m.diagnostic_events.iter().collect())
            .unwrap_or_default(),
        TransactionMeta::V4(v4) => v4.diagnostic_events.iter().collect(),
        _ => Vec::new(),
    };

    let mut per_contract: BTreeMap<String, u32> = BTreeMap::new();
    for diag in diags {
        if !matches!(diag.event.type_, ContractEventType::Diagnostic) {
            continue;
        }
        let ContractEventBody::V0(ref v0) = diag.event.body;
        if v0.topics.len() < 2 {
            continue;
        }
        let head = match v0.topics.first() {
            Some(ScVal::Symbol(s)) => std::str::from_utf8(s.as_vec()).unwrap_or(""),
            _ => continue,
        };
        if head != "fn_call" {
            continue;
        }
        let strkey = match v0.topics.get(1) {
            Some(ScVal::Bytes(b)) if b.as_slice().len() == 32 => {
                let mut buf = [0u8; 32];
                buf.copy_from_slice(b.as_slice());
                ScAddress::Contract(ContractId(Hash(buf))).to_string()
            }
            Some(ScVal::Address(addr @ ScAddress::Contract(_))) => addr.to_string(),
            _ => continue,
        };
        *per_contract.entry(strkey).or_insert(0) += 1;
    }

    let total: u32 = per_contract.values().sum();
    println!("{{");
    println!(
        "  \"meta_variant\": \"{}\",",
        match meta {
            TransactionMeta::V3(_) => "V3",
            TransactionMeta::V4(_) => "V4",
            _ => "other",
        }
    );
    println!("  \"total_fn_calls\": {},", total);
    println!("  \"distinct_contracts\": {},", per_contract.len());
    println!("  \"per_contract\": {{");
    let entries: Vec<_> = per_contract.iter().collect();
    for (i, (cid, n)) in entries.iter().enumerate() {
        let comma = if i + 1 < entries.len() { "," } else { "" };
        println!("    \"{}\": {}{}", cid, n, comma);
    }
    println!("  }}");
    println!("}}");
}
