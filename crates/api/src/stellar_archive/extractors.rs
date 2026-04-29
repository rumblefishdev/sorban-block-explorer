//! Thin wrappers over `xdr_parser::extract_*` that pick out the heavy-field
//! subset each endpoint needs.
//!
//! The wrappers are **pure functions** on already-fetched `LedgerCloseMeta`
//! — no I/O, no DB, no state. They slice the parsed ledger down to the
//! heavy-payload shape defined in `dto.rs`.
//!
//! Endpoints (wired in a follow-up task) call these after `StellarArchiveFetcher` hands
//! back the `LedgerCloseMeta` from the public archive.

use stellar_xdr::curr::{LedgerCloseMeta, TransactionEnvelope, TransactionMeta};
use tracing::instrument;

use super::dto::{E3HeavyFields, E14HeavyEventFields, SignatureDto, XdrEventDto, XdrOperationDto};

/// Extract the heavy-field subset of the E3 (`/transactions/:hash`) response
/// for a given transaction hash within the supplied ledger.
///
/// `tx_hash` is the lowercase hex (64 chars) transaction hash — same format
/// as `ExtractedTransaction.hash` produced by `xdr_parser::extract_transactions`.
///
/// Returns `None` when the ledger does not contain a transaction matching
/// `tx_hash`. The calling handler should treat that as "no heavy fields
/// available" and fall back to DB-only response with
/// `heavy_fields_status: unavailable`.
#[instrument(skip(meta, network_id), fields(tx_hash = %tx_hash))]
pub fn extract_e3_heavy(
    meta: &LedgerCloseMeta,
    tx_hash: &str,
    network_id: &[u8; 32],
) -> Option<E3HeavyFields> {
    let ledger = xdr_parser::extract_ledger(meta);
    let ledger_seq = ledger.sequence;
    let closed_at = ledger.closed_at;

    let extracted_txs = xdr_parser::extract_transactions(meta, ledger_seq, closed_at, network_id);
    let envelopes = xdr_parser::envelope::extract_envelopes(meta, network_id);
    let tx_metas = collect_tx_metas(meta);

    let (idx, ext_tx) = extracted_txs
        .iter()
        .enumerate()
        .find(|(_, t)| t.hash == tx_hash)?;

    let envelope = envelopes.get(idx).and_then(Option::as_ref);
    let tx_meta = tx_metas.get(idx).copied();

    // Envelope-level details: signatures + fee-bump source.
    let (signatures, fee_bump_source) = envelope
        .map(|env| (envelope_signatures(env), envelope_fee_bump_source(env)))
        .unwrap_or_default();

    // Events: call extract_events if we have tx meta; returns contract + diagnostic together.
    let (contract_events, diagnostic_events) = match tx_meta {
        Some(tm) => split_events(xdr_parser::extract_events(
            tm,
            &ext_tx.hash,
            ledger_seq,
            closed_at,
        )),
        None => (Vec::new(), Vec::new()),
    };

    // Invocations: nested Soroban call tree (flat list is not exposed by any endpoint).
    let operation_tree = match (envelope, tx_meta) {
        (Some(env), Some(tm)) => {
            let inner = xdr_parser::envelope::inner_transaction(env);
            xdr_parser::extract_invocations(
                &inner,
                Some(tm),
                &ext_tx.hash,
                ledger_seq,
                closed_at,
                &ext_tx.source_account,
                ext_tx.successful,
            )
            .operation_tree
        }
        _ => None,
    };

    // Operations: raw details per op.
    let operations = envelope
        .map(|env| {
            let inner = xdr_parser::envelope::inner_transaction(env);
            xdr_parser::extract_operations(&inner, tx_meta, &ext_tx.hash, ledger_seq, idx)
                .into_iter()
                .filter_map(to_operation_dto)
                .collect()
        })
        .unwrap_or_default();

    Some(E3HeavyFields {
        memo_type: ext_tx.memo_type.clone(),
        memo: ext_tx.memo.clone(),
        signatures,
        fee_bump_source,
        envelope_xdr: Some(ext_tx.envelope_xdr.clone()).filter(|s| !s.is_empty()),
        result_xdr: Some(ext_tx.result_xdr.clone()).filter(|s| !s.is_empty()),
        diagnostic_events,
        contract_events,
        operations,
        result_code: if ext_tx.parse_error {
            None
        } else {
            Some(ext_tx.result_code.clone())
        },
        operation_tree,
    })
}

/// Extract the heavy-field subset of the E14 (`/contracts/:id/events`) response:
/// full `topics[0..N]` + decoded `data` for every event emitted by `contract_id`
/// within the supplied ledger.
///
/// `contract_id` is the StrKey C… address (56 chars).
#[allow(dead_code)] // used by future E14 events endpoint
#[instrument(skip(meta, network_id), fields(contract_id = %contract_id, events = tracing::field::Empty))]
pub fn extract_e14_heavy(
    meta: &LedgerCloseMeta,
    contract_id: &str,
    network_id: &[u8; 32],
) -> Vec<E14HeavyEventFields> {
    let ledger = xdr_parser::extract_ledger(meta);
    let ledger_seq = ledger.sequence;
    let closed_at = ledger.closed_at;

    let extracted_txs = xdr_parser::extract_transactions(meta, ledger_seq, closed_at, network_id);
    let tx_metas = collect_tx_metas(meta);

    let mut out = Vec::new();
    for (idx, ext_tx) in extracted_txs.iter().enumerate() {
        let Some(tm) = tx_metas.get(idx).copied() else {
            continue;
        };
        let events = xdr_parser::extract_events(tm, &ext_tx.hash, ledger_seq, closed_at);
        for event in events {
            if event.contract_id.as_deref() == Some(contract_id) {
                let Some(event_index) = to_i16_index(event.event_index, "event_index") else {
                    continue;
                };
                let topics = topics_to_vec(event.topics);
                out.push(E14HeavyEventFields {
                    event_index,
                    transaction_hash: event.transaction_hash,
                    topics,
                    data: event.data,
                });
            }
        }
    }

    tracing::Span::current().record("events", out.len() as u64);
    out
}

// --- private helpers ---

/// Checked `u32 → i16` conversion for indices that correlate to DB `SMALLINT`
/// columns (`event_index`, `invocation_index`, `application_order`).
/// Returns `None` and logs a warning if the value overflows i16 — the caller
/// skips the row rather than silently truncate and corrupt correlation with DB.
fn to_i16_index(value: u32, kind: &'static str) -> Option<i16> {
    match i16::try_from(value) {
        Ok(v) => Some(v),
        Err(_) => {
            tracing::warn!(
                kind,
                value,
                "index out of SMALLINT range — skipping row to avoid silent truncation"
            );
            None
        }
    }
}

/// Collect borrowed `&TransactionMeta` references for every transaction in
/// the ledger, in the same order as `tx_processing` (i.e. index `i` in the
/// returned `Vec` corresponds to the `i`-th entry of `tx_processing`, which
/// also matches `xdr_parser::extract_transactions` and
/// `xdr_parser::envelope::extract_envelopes` output ordering — callers rely
/// on this alignment when joining metas back to extracted txs by index).
/// Mirrors the unified collection used in
/// `crates/indexer/src/handler/process.rs::collect_tx_metas`.
///
/// `pub` (rather than `pub(super)`) so per-endpoint modules outside
/// `stellar_archive` (E13/E14 in `contracts/`) can re-extract per-tx
/// metadata without a parallel implementation.
pub fn collect_tx_metas(meta: &LedgerCloseMeta) -> Vec<&TransactionMeta> {
    match meta {
        LedgerCloseMeta::V0(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
        LedgerCloseMeta::V1(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
        LedgerCloseMeta::V2(v) => v
            .tx_processing
            .iter()
            .map(|p| &p.tx_apply_processing)
            .collect(),
    }
}

fn envelope_signatures(env: &TransactionEnvelope) -> Vec<SignatureDto> {
    let sigs: &[stellar_xdr::curr::DecoratedSignature] = match env {
        TransactionEnvelope::TxV0(v0) => &v0.signatures,
        TransactionEnvelope::Tx(v1) => &v1.signatures,
        TransactionEnvelope::TxFeeBump(fb) => &fb.signatures,
    };
    sigs.iter()
        .map(|s| SignatureDto {
            hint: hex::encode(s.hint.0),
            signature: hex::encode(&s.signature.0),
        })
        .collect()
}

fn envelope_fee_bump_source(env: &TransactionEnvelope) -> Option<String> {
    match env {
        TransactionEnvelope::TxFeeBump(fb) => Some(fb.tx.fee_source.to_string()),
        _ => None,
    }
}

fn split_events(events: Vec<xdr_parser::ExtractedEvent>) -> (Vec<XdrEventDto>, Vec<XdrEventDto>) {
    use xdr_parser::EventSource;

    // Route on container source, not inner `event_type` — the
    // diagnostic_events container holds byte-identical Contract-typed
    // copies of per-op consensus events (inner `type_ = Contract`) when
    // diagnostic mode is enabled, so a type-based split would surface
    // those copies as additional contract events (task 0182).
    let mut contract = Vec::new();
    let mut diagnostic = Vec::new();
    for e in events {
        let Some(event_index) = to_i16_index(e.event_index, "event_index") else {
            continue;
        };
        let is_diagnostic = e.source == EventSource::Diagnostic;
        let topics = topics_to_vec(e.topics);
        let dto = XdrEventDto {
            event_type: e.event_type.to_string(),
            contract_id: e.contract_id,
            topics,
            data: e.data,
            event_index,
        };
        if is_diagnostic {
            diagnostic.push(dto);
        } else {
            contract.push(dto);
        }
    }
    (contract, diagnostic)
}

fn topics_to_vec(topics: serde_json::Value) -> Vec<serde_json::Value> {
    match topics {
        serde_json::Value::Array(a) => a,
        other => vec![other],
    }
}

fn to_operation_dto(op: xdr_parser::ExtractedOperation) -> Option<XdrOperationDto> {
    let application_order = to_i16_index(op.operation_index, "application_order")?;
    Some(XdrOperationDto {
        op_type: op.op_type.to_string(),
        application_order,
        details: op.details,
    })
}
