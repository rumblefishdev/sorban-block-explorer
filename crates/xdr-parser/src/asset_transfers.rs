//! Edge extraction for the `asset_transfers` table (task 0540): one row per
//! token movement, decoded from the consensus per-operation events.
//!
//! This module is the single decode both the live indexer and the S3
//! backfill run, so the two write byte-identical rows. Three rules it
//! enforces, each measured before it was written (README 0540, "Review by a
//! second pass"):
//!
//! 1. **The asset is the emitter, not the topic string.** A labelled event
//!    (`"USDC:G…"` in the last topic) is accepted only if the emitting contract
//!    IS that asset's Stellar Asset Contract — `emitter == derive_sac(asset)`.
//!    The protocol validates the emitter, never the content, so without this
//!    gate any contract could put "USDC" on a victim's account page. Measured
//!    on 60 000 ledgers: 25 912 of 25 912 labelled (emitter, asset) pairs pass.
//! 2. **An amount is a scalar `i128`/`u128`, or the `amount` key of a map;
//!    `token_id` means non-fungible.** Anything else is a protocol annotation
//!    (`{mint_amount, mint_tokens}` restating a mint that has its own event),
//!    never a movement: it is rejected AND counted, so a new shape surfaces as
//!    a number rather than a silent zero.
//! 3. **Only the per-operation container.** Diagnostics are byte-identical
//!    copies (or rolled-back calls), and token verbs never appear at
//!    transaction level (12 237 of 12 237 measured) — one that does is a
//!    reject, not a row with a null operation.
//!
//! Rejects are returned to the caller, which raises them as ingest errors;
//! they are a developer's problem, not something a reader of the account page
//! can act on, so they never become rows.

use serde_json::Value;
use tracing::warn;

use crate::event_filters::{EventAsset, TokenEventKind, parse_token_event};
use crate::sac::sac_override_from_event_topics;
use crate::types::{EventSource, ExtractedEvent};

/// What a token event's `data` payload says about the amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenAmount {
    /// A fungible amount in the token's own base units.
    Fungible(i128),
    /// A non-fungible movement (`{token_id}`): complete, and has no amount.
    NonFungible,
    /// Not a movement we recognise — rejected and counted by the caller.
    Unrecognised,
}

/// Read the amount out of a token event's ScVal-decoded `data`.
///
/// Shapes, all measured on production (task 0540 T06): scalar `i128` (the
/// common case), `map{amount, to_muxed_id}` (muxed / memo-carrying
/// payments), `map{amount, amount0, amount1, …}` (a concentrated-liquidity
/// position mint — `amount` is the position, the other two are components,
/// not movements), `map{token_id}` (non-fungible). The map is read **by key**,
/// never positionally.
pub fn token_event_amount(data: &Value) -> TokenAmount {
    match data.get("type").and_then(Value::as_str) {
        Some("i128") | Some("u128") => match scalar_i128(data) {
            Some(n) => TokenAmount::Fungible(n),
            None => TokenAmount::Unrecognised,
        },
        Some("map") => {
            let entries = data.get("value").and_then(Value::as_array);
            let Some(entries) = entries else {
                return TokenAmount::Unrecognised;
            };
            let field = |name: &str| {
                entries.iter().find_map(|e| {
                    let key = e.get("key")?;
                    (key.get("type").and_then(Value::as_str) == Some("sym")
                        && key.get("value").and_then(Value::as_str) == Some(name))
                    .then(|| e.get("value"))
                    .flatten()
                })
            };
            if let Some(amount) = field("amount") {
                return match scalar_i128(amount) {
                    Some(n) => TokenAmount::Fungible(n),
                    None => TokenAmount::Unrecognised,
                };
            }
            if field("token_id").is_some() {
                return TokenAmount::NonFungible;
            }
            TokenAmount::Unrecognised
        }
        _ => TokenAmount::Unrecognised,
    }
}

/// An `i128` / `u128` typed-JSON scalar (`{"type":"i128","value":"123"}`).
/// A `u128` above `i128::MAX` cannot be stored and reads as unrecognised.
fn scalar_i128(v: &Value) -> Option<i128> {
    match v.get("type").and_then(Value::as_str)? {
        "i128" | "u128" => v.get("value").and_then(Value::as_str)?.parse::<i128>().ok(),
        _ => None,
    }
}

/// One token movement, decoded — the parser-side row of `asset_transfers`.
/// Addresses are the StrKeys the event carried; the persistence layer resolves
/// them to surrogates and splits an `M…` into its `G…` and id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedAssetTransfer {
    pub transaction_hash: String,
    /// Our flat per-transaction counter (joins `soroban_events`).
    pub event_index: u32,
    /// Official identity: envelope position of the emitting operation…
    pub op_index: u32,
    /// …and the event's position within that operation's event list.
    pub event_pos_in_op: u32,
    pub kind: TokenEventKind,
    /// `None` for `mint`.
    pub from: Option<String>,
    /// `None` for `burn` and `clawback`.
    pub to: Option<String>,
    pub asset: EventAsset,
    /// The emitting contract (`C…`). For a bespoke token this IS the asset.
    pub emitter: String,
    /// `None` has exactly one meaning: a non-fungible movement.
    pub amount: Option<i128>,
}

/// Why an event with a token verb did not become a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferReject {
    /// A labelled event whose emitter is not the asset's derived SAC — the
    /// spoofing shape. `asset` is the label the event claimed.
    EmitterNotSac {
        transaction_hash: String,
        event_index: u32,
        emitter: String,
        asset: String,
    },
    /// A token verb with a payload that is neither an amount nor a token id.
    UnrecognisedPayload {
        transaction_hash: String,
        event_index: u32,
        emitter: String,
        kind: TokenEventKind,
        data_type: String,
    },
    /// A token verb outside the per-operation container — measured never to
    /// happen; if it does, the official identity is undefined for it.
    NoOperation {
        transaction_hash: String,
        event_index: u32,
        emitter: String,
    },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AssetTransferExtraction {
    pub transfers: Vec<ExtractedAssetTransfer>,
    pub rejects: Vec<TransferReject>,
}

/// Decode every token movement in one transaction's events.
///
/// Diagnostic-container events are skipped silently (byte-identical copies of
/// consensus events, or the trace of a call that was rolled back — task 0182,
/// re-measured in 0540). Events that are not token events are skipped
/// silently. Everything that *is* a token verb either becomes a transfer or a
/// reject; nothing with a token verb is dropped without a trace.
pub fn extract_asset_transfers(
    events: &[ExtractedEvent],
    net_id: &[u8; 32],
) -> AssetTransferExtraction {
    let mut out = AssetTransferExtraction::default();
    for ev in events {
        if ev.source == EventSource::Diagnostic {
            continue;
        }
        let Some(token) = parse_token_event(&ev.topics) else {
            continue;
        };
        // Staging drops events with no emitting contract; a token verb without
        // one has never been observed (0 of 12 237). Treat it like a missing
        // operation: it cannot be a row, and it must not vanish.
        let emitter = ev.contract_id.clone().unwrap_or_default();

        let (Some(op_index), Some(event_pos_in_op)) = (ev.op_index, ev.event_pos_in_op) else {
            warn!(
                target: "xdr_parser::asset_transfers",
                tx = %ev.transaction_hash, event_index = ev.event_index, %emitter,
                "token verb outside the per-operation container — rejected"
            );
            out.rejects.push(TransferReject::NoOperation {
                transaction_hash: ev.transaction_hash.clone(),
                event_index: ev.event_index,
                emitter,
            });
            continue;
        };

        // Rule 1 — a labelled asset must be emitted by its own SAC.
        if !matches!(token.asset, EventAsset::Bespoke)
            && sac_override_from_event_topics(&emitter, &ev.topics, net_id).is_none()
        {
            let asset = ev
                .topics
                .as_array()
                .and_then(|t| t.last())
                .and_then(|t| t.get("value"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            warn!(
                target: "xdr_parser::asset_transfers",
                tx = %ev.transaction_hash, event_index = ev.event_index, %emitter, %asset,
                "labelled token event whose emitter is not the asset's SAC — rejected"
            );
            out.rejects.push(TransferReject::EmitterNotSac {
                transaction_hash: ev.transaction_hash.clone(),
                event_index: ev.event_index,
                emitter,
                asset,
            });
            continue;
        }

        // Rule 2 — the payload is an amount, a token id, or not a movement.
        let amount = match token_event_amount(&ev.data) {
            TokenAmount::Fungible(n) => Some(n),
            TokenAmount::NonFungible => None,
            TokenAmount::Unrecognised => {
                let data_type = ev
                    .data
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                warn!(
                    target: "xdr_parser::asset_transfers",
                    tx = %ev.transaction_hash, event_index = ev.event_index, %emitter,
                    kind = ?token.kind, %data_type,
                    "token verb with an unrecognised payload — rejected, not a movement"
                );
                out.rejects.push(TransferReject::UnrecognisedPayload {
                    transaction_hash: ev.transaction_hash.clone(),
                    event_index: ev.event_index,
                    emitter,
                    kind: token.kind,
                    data_type,
                });
                continue;
            }
        };

        out.transfers.push(ExtractedAssetTransfer {
            transaction_hash: ev.transaction_hash.clone(),
            event_index: ev.event_index,
            op_index,
            event_pos_in_op,
            kind: token.kind,
            from: token.from,
            to: token.to,
            asset: token.asset,
            emitter,
            amount,
        });
    }
    out
}

#[cfg(test)]
#[path = "asset_transfers_tests.rs"]
mod tests;
