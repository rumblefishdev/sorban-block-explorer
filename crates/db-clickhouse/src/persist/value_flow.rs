//! Staging for the value-flow tables (task 0540 / 0541): `asset_transfers`,
//! `transaction_memos`, `soroban_event_ops`.
//!
//! The parser has already decided *what* moved (`xdr_parser::extract_asset_transfers`:
//! emitter gate, payload shapes, official identity). This module only turns
//! StrKeys into surrogates and attaches the two things a transfer row needs
//! from the envelope rather than from the event:
//!
//! - the transaction's `application_order` (its position in the ledger);
//! - the multiplexing ids. CAP-67 keeps `to` as the underlying `G…` and puts
//!   the id (merged with the memo) in the event data; the envelope carries the
//!   `M…` destination and source unambiguously, so the ids come from there,
//!   matched to the transfer through `op_index`. A topic that *is* an `M…`
//!   (Soroban contracts may pass one) is split here the same way.
//!
//! Lives in its own file because `stage.rs` is past the module size limit;
//! `prepare_with_sac_overrides` calls [`build_value_flow_rows`] once per ledger.

use std::collections::HashMap;

use serde_json::Value;
use xdr_parser::types::{ExtractedEvent, ExtractedOperation, ExtractedTransaction};
use xdr_parser::{EventAsset, EventSource, ExtractedAssetTransfer, TokenEventKind};

use super::ids;
use super::rows::{AssetTransferRow, SorobanEventOpRow, TransactionMemoRow};
use super::stage::event_asset_surrogate;
use crate::SchemaError;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ValueFlowRows {
    pub transfers: Vec<AssetTransferRow>,
    pub memos: Vec<TransactionMemoRow>,
    pub event_ops: Vec<SorobanEventOpRow>,
}

/// Per-transaction facts the edge rows borrow from the envelope.
struct TxFacts<'a> {
    application_order: i16,
    source_account: &'a str,
    source_muxed_id: Option<u64>,
}

pub fn build_value_flow_rows(
    ledger_sequence: i64,
    transactions: &[ExtractedTransaction],
    operations: &[(String, Vec<ExtractedOperation>)],
    events: &[(String, Vec<ExtractedEvent>)],
    transfers: &[ExtractedAssetTransfer],
) -> Result<ValueFlowRows, SchemaError> {
    let mut out = ValueFlowRows::default();

    let mut tx_by_hash: HashMap<&str, TxFacts<'_>> = HashMap::with_capacity(transactions.len());
    for (idx, tx) in transactions.iter().enumerate() {
        let application_order = i16::try_from(idx + 1)
            .map_err(|_| SchemaError::Staging("application_order overflow (>i16)".into()))?;
        tx_by_hash.insert(
            tx.hash.as_str(),
            TxFacts {
                application_order,
                source_account: &tx.source_account,
                source_muxed_id: tx.source_muxed_id,
            },
        );
        if let (Some(memo_type), Some(memo)) = (&tx.memo_type, &tx.memo) {
            out.memos.push(TransactionMemoRow {
                ledger_sequence,
                application_order,
                memo_type: memo_type.clone(),
                memo: memo.clone(),
            });
        }
    }

    let ops_by_hash: HashMap<&str, &[ExtractedOperation]> = operations
        .iter()
        .map(|(hash, ops)| (hash.as_str(), ops.as_slice()))
        .collect();

    for t in transfers {
        let Some(tx) = tx_by_hash.get(t.transaction_hash.as_str()) else {
            return Err(SchemaError::Staging(format!(
                "asset transfer for unknown transaction {}",
                t.transaction_hash
            )));
        };
        // `operation_index` is 1-based (Horizon convention); `op_index` is the
        // zero-based envelope position the event container reports.
        let op = ops_by_hash
            .get(t.transaction_hash.as_str())
            .and_then(|ops| ops.get(t.op_index as usize))
            .filter(|op| op.operation_index == t.op_index + 1);

        let (from_id, from_kind, from_muxed_id) = endpoint(t.from.as_deref(), |g| {
            // Sender: the op's own source if it overrides, else the tx source.
            match op {
                Some(op) if op.source_account.is_some() => {
                    (op.source_account.as_deref() == Some(g)).then_some(op.source_muxed_id)
                }
                _ => (tx.source_account == g).then_some(tx.source_muxed_id),
            }
            .flatten()
        });
        let (to_id, to_kind, to_muxed_id) = endpoint(t.to.as_deref(), |g| {
            // Recipient: the op's own destination — and only for the transfer
            // that delivers the op's asset to it. A path payment that crosses
            // the recipient's own offer moves other assets to the same address
            // inside the same op, and an arbitrage with source == destination
            // moves several; none of those is the exchange deposit the
            // sub-account id names (deep review, 2026-09-07).
            op.filter(|op| op.details.get("destination").and_then(Value::as_str) == Some(g))
                .filter(|op| op_delivers(op, &t.asset))
                .and_then(|op| op.destination_muxed_id)
        });

        let emitter_id = ids::contract_id(&t.emitter);
        let asset_id = event_asset_surrogate(&t.asset, Some(emitter_id)).ok_or_else(|| {
            SchemaError::Staging(format!(
                "asset transfer without a resolvable asset (tx {}, event {})",
                t.transaction_hash, t.event_index
            ))
        })?;

        out.transfers.push(AssetTransferRow {
            ledger_sequence,
            application_order: tx.application_order,
            op_index: narrow(t.op_index, "op_index")?,
            event_pos_in_op: narrow(t.event_pos_in_op, "event_pos_in_op")?,
            event_index: narrow(t.event_index, "event_index")?,
            asset_id,
            amount: t.amount,
            from_id,
            from_kind,
            from_muxed_id,
            to_id,
            to_kind,
            to_muxed_id,
            verb: verb(t.kind).to_string(),
        });
    }

    for (hash, evs) in events {
        let Some(tx) = tx_by_hash.get(hash.as_str()) else {
            continue;
        };
        for ev in evs {
            if ev.source == EventSource::Diagnostic {
                continue;
            }
            let (Some(op_index), Some(pos)) = (ev.op_index, ev.event_pos_in_op) else {
                continue; // tx-level: no operation, no row (0541)
            };
            out.event_ops.push(SorobanEventOpRow {
                ledger_sequence,
                application_order: tx.application_order,
                event_index: narrow(ev.event_index, "event_index")?,
                op_index: narrow(op_index, "op_index")?,
                event_pos_in_op: narrow(pos, "event_pos_in_op")?,
            });
        }
    }

    Ok(out)
}

/// Does this operation deliver `asset` to its destination? Payment: `asset`;
/// path payments: `destAsset`; AccountMerge carries no asset in its details
/// and moves native. A bespoke token is never what a classic operation
/// delivers.
fn op_delivers(op: &ExtractedOperation, asset: &EventAsset) -> bool {
    let delivered = op
        .details
        .get("destAsset")
        .or_else(|| op.details.get("asset"))
        .and_then(Value::as_str);
    match (delivered, asset) {
        (None | Some("native"), EventAsset::Native) => true,
        (Some(s), EventAsset::Credit { code, issuer }) => {
            s.split_once(':') == Some((code.as_str(), issuer.as_str()))
        }
        _ => false,
    }
}

/// Resolve one end of a transfer: `(surrogate, kind, muxed id)`.
///
/// `kind` is the StrKey's first letter — `G` account, `C` contract, `L` classic
/// pool, `B` claimable balance — which tells the reader which table resolves
/// the id. An `M…` is split into its `G…` (hashed, so the account page finds
/// it) and its id; otherwise the id comes from the envelope via `from_envelope`.
fn endpoint(
    strkey: Option<&str>,
    from_envelope: impl FnOnce(&str) -> Option<u64>,
) -> (Option<i64>, String, Option<u64>) {
    let Some(s) = strkey else {
        return (None, String::new(), None);
    };
    let (g, topic_muxed_id) = split_muxed(s);
    let muxed_id = topic_muxed_id.or_else(|| from_envelope(&g));
    let kind = g.chars().next().map(|c| c.to_string()).unwrap_or_default();
    (Some(ids::address_id(&g)), kind, muxed_id)
}

/// `M…` → (`G…`, id); anything else unchanged.
fn split_muxed(strkey: &str) -> (String, Option<u64>) {
    if !strkey.starts_with('M') {
        return (strkey.to_string(), None);
    }
    match stellar_strkey::Strkey::from_string(strkey) {
        Ok(stellar_strkey::Strkey::MuxedAccountEd25519(m)) => {
            let g = stellar_strkey::Strkey::PublicKeyEd25519(stellar_strkey::ed25519::PublicKey(
                m.ed25519,
            ));
            (g.to_string().as_str().to_owned(), Some(m.id))
        }
        _ => (strkey.to_string(), None),
    }
}

fn verb(kind: TokenEventKind) -> &'static str {
    match kind {
        TokenEventKind::Transfer => "transfer",
        TokenEventKind::Mint => "mint",
        TokenEventKind::Burn => "burn",
        TokenEventKind::Clawback => "clawback",
    }
}

fn narrow(v: u32, what: &str) -> Result<i16, SchemaError> {
    i16::try_from(v).map_err(|_| SchemaError::Staging(format!("{what} overflow (>i16): {v}")))
}

#[cfg(test)]
#[path = "value_flow_tests.rs"]
mod tests;
