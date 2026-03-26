---
source: rumblefishdev/stellar-indexer (private repo)
file: src/events/mod.rs
retrieved: 2026-03-26
note: Copied from local checkout. Shows V3 vs V4 event handling pattern.
---

# stellar-indexer: src/events/mod.rs

Event extraction from TransactionMeta. Critical reference for V3 → V4 migration.

Key patterns:

- V3: events in `soroban_meta.events`
- V4: events at top-level `v4.events` (with stage) + per-operation `op_meta.events`
- V4: diagnostic events also at top-level
- Skip diagnostic events without contractId

```rust
use serde_json::json;
use stellar_xdr::curr::{ContractEvent, ContractEventBody, ContractEventType, TransactionMeta};

use crate::scval::scval_to_typed_json_value;

pub fn skip_diagnostic_without_contract(ev: &ContractEvent) -> bool {
    ev.type_ == ContractEventType::Diagnostic && ev.contract_id.is_none()
}

pub fn contract_event_to_json(
    ev: &ContractEvent,
    stage: Option<&str>,
    in_successful_contract_call: Option<bool>,
) -> serde_json::Value {
    let contract_str = ev
        .contract_id
        .as_ref()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "<no contract>".to_string());
    let (data_value, topics_value) = {
        let ContractEventBody::V0(v0) = &ev.body;
        (
            scval_to_typed_json_value(&v0.data),
            json!(v0
                .topics
                .iter()
                .map(scval_to_typed_json_value)
                .collect::<Vec<_>>()),
        )
    };
    let mut obj = json!({
        "type": ev.type_.name(),
        "contract": contract_str,
        "data": data_value,
    });
    if let Some(arr) = topics_value.as_array() {
        if !arr.is_empty() {
            obj["topics"] = topics_value;
        }
    }
    if let Some(s) = stage {
        obj["stage"] = json!(s);
    }
    if let Some(b) = in_successful_contract_call {
        obj["in_successful_contract_call"] = json!(b);
    }
    obj
}

pub fn events_from_tx_meta_to_json(tx_meta: &TransactionMeta) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    match tx_meta {
        TransactionMeta::V3(v3) => {
            if let Some(ref soroban) = v3.soroban_meta {
                for ev in soroban.events.iter() {
                    out.push(contract_event_to_json(ev, None, None));
                }
                for de in soroban.diagnostic_events.iter() {
                    if skip_diagnostic_without_contract(&de.event) {
                        continue;
                    }
                    out.push(contract_event_to_json(
                        &de.event,
                        None,
                        Some(de.in_successful_contract_call),
                    ));
                }
            }
        }
        TransactionMeta::V4(v4) => {
            for te in v4.events.iter() {
                out.push(contract_event_to_json(&te.event, Some(te.stage.name()), None));
            }
            for op_meta in v4.operations.iter() {
                for ev in op_meta.events.iter() {
                    out.push(contract_event_to_json(ev, None, None));
                }
            }
            for de in v4.diagnostic_events.iter() {
                if skip_diagnostic_without_contract(&de.event) {
                    continue;
                }
                out.push(contract_event_to_json(
                    &de.event,
                    None,
                    Some(de.in_successful_contract_call),
                ));
            }
        }
        _ => {}
    }
    out
}
```
