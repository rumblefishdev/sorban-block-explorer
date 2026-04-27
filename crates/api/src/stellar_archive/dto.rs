//! DTOs for XDR-sourced fields, composed with DB-sourced light slices to
//! produce final E3/E14 endpoint responses.
//!
//! Per ADR 0027 Part III + ADR 0029 + ADR 0033: the DB stores identity and
//! index columns; event, memo, signature, and envelope detail lives only on
//! the public Stellar archive. For E14 this means the entire event payload
//! (type, topics, data, event index) is S3-sourced — there is no DB-side
//! event row to merge against. E3 still carries its DB tx-light slice and
//! composes it with an XDR heavy struct via `merge_e3_response`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// E3 (`GET /transactions/:hash`) — fields sourced from XDR parse.
///
/// Returned as the `heavy` block inside `E3Response<TransactionDetailLight>`.
/// On upstream fetch failure the caller substitutes `None` and
/// `merge_e3_response` sets `heavy_fields_status = "unavailable"`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct E3HeavyFields {
    /// Memo type as ASCII tag (`"text"`, `"id"`, `"hash"`, `"return"`, `"none"`).
    pub memo_type: Option<String>,
    /// Memo payload rendered as string (hex for hash/return, decimal for id).
    pub memo: Option<String>,
    /// Transaction-level signatures (hint + signature bytes hex-encoded).
    pub signatures: Vec<SignatureDto>,
    /// Fee-bump envelope `feeSource` StrKey when the outer tx is a fee-bump.
    pub fee_bump_source: Option<String>,
    /// Base64-encoded `TransactionEnvelope`.
    pub envelope_xdr: Option<String>,
    /// Base64-encoded `TransactionResult`.
    pub result_xdr: Option<String>,
    /// Diagnostic events emitted during Soroban invocation.
    pub diagnostic_events: Vec<XdrEventDto>,
    /// Non-diagnostic Soroban events (contract + system) with full topic
    /// array + decoded data payload.
    pub contract_events: Vec<XdrEventDto>,
    /// Operations with full XDR-decoded details (type-specific JSON).
    pub operations: Vec<XdrOperationDto>,
    /// Transaction result code (e.g. `"txSuccess"`, `"txFailed"`).
    /// `None` only when the transaction had a parse error.
    pub result_code: Option<String>,
    /// Nested Soroban invocation tree, derived from `result_meta_xdr` at
    /// extraction time (the raw `result_meta_xdr` itself is intentionally
    /// not surfaced — see 0046 spec "result_meta_xdr is NOT returned").
    pub operation_tree: Option<serde_json::Value>,
}

/// E14 (`GET /contracts/:id/events`) — per-event payload materialised from
/// the ledger XDR.
///
/// ADR 0033: the DB appearance index only tells us *which* `(contract, tx,
/// ledger)` trios carry events and how many. The actual event payload (type,
/// topics, data, per-event index within the tx) is extracted from the
/// public-archive XDR at request time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E14HeavyEventFields {
    /// Event index within its transaction. Stable per-tx identifier that
    /// survives across requests — used for client-side de-duplication when a
    /// page redraw overlaps a previous page.
    pub event_index: i16,
    /// Transaction hash (hex) this event belongs to — needed because a
    /// single ledger's events may span many transactions.
    pub transaction_hash: String,
    /// Full topics array as decoded JSON.
    pub topics: Vec<serde_json::Value>,
    /// Event data payload as decoded JSON.
    pub data: serde_json::Value,
}

/// Single signature on a transaction envelope.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SignatureDto {
    /// 4-byte hint (lowercase hex, 8 chars).
    pub hint: String,
    /// Signature bytes (lowercase hex).
    pub signature: String,
}

/// Common shape for contract events and diagnostic events.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct XdrEventDto {
    /// `"contract"`, `"system"`, or `"diagnostic"`.
    pub event_type: String,
    /// Contract address (StrKey) that emitted the event, if any.
    pub contract_id: Option<String>,
    /// Decoded topic array.
    pub topics: Vec<serde_json::Value>,
    /// Decoded event data payload.
    pub data: serde_json::Value,
    /// Event index within the transaction (zero-based).
    pub event_index: i16,
}

/// Operation raw parameters (XDR-decoded full details).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct XdrOperationDto {
    /// Operation type tag (e.g. `"payment"`, `"invoke_host_function"`).
    pub op_type: String,
    /// Application order within the transaction (zero-based).
    pub application_order: i16,
    /// Full operation details (type-specific JSON).
    pub details: serde_json::Value,
}

/// Merged E3 response: DB light fields + optional XDR heavy fields.
///
/// `heavy_fields_status` = `Ok` when `heavy` is `Some`, `Unavailable` when
/// the public-archive fetch failed and the caller degraded gracefully.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct E3Response<TxLight> {
    #[serde(flatten)]
    #[schema(inline)]
    pub light: TxLight,
    pub heavy: Option<E3HeavyFields>,
    pub heavy_fields_status: HeavyFieldsStatus,
}

/// Indicates whether the XDR-sourced fields were loaded successfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeavyFieldsStatus {
    Ok,
    Unavailable,
}
