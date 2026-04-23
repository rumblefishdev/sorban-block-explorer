//! DTOs for XDR-sourced fields, composed with DB-sourced light slices to
//! produce final E3/E14 endpoint responses.
//!
//! Per ADR 0027 Part III + ADR 0029 + ADR 0033: the DB stores identity and
//! index columns; event, memo, signature, and envelope detail lives only on
//! the public Stellar archive. For E14 this means the entire event payload
//! (type, topics, data, event index) is S3-sourced — there is no DB-side
//! event row to merge against. E3 still carries its DB tx-light slice and
//! composes it with an XDR heavy struct.

use serde::{Deserialize, Serialize};

/// E3 (`GET /transactions/:hash`) — fields sourced from XDR parse.
///
/// All optional: a tx that parses cleanly exposes every field; on upstream
/// failure the caller substitutes `None` and sets `heavy_fields_status` on
/// the merged response.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Base64-encoded `TransactionMeta`.
    pub result_meta_xdr: Option<String>,
    /// Diagnostic events emitted during Soroban invocation.
    pub diagnostic_events: Vec<XdrEventDto>,
    /// Non-diagnostic Soroban events (contract + system) with full topic
    /// array + decoded data payload.
    pub contract_events: Vec<XdrEventDto>,
    /// Soroban invocations with full function args + return value payloads.
    pub invocations: Vec<XdrInvocationDto>,
    /// Operation raw parameters (full JSON details, not just indexed columns).
    pub operations: Vec<XdrOperationDto>,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureDto {
    /// 4-byte hint (lowercase hex, 8 chars).
    pub hint: String,
    /// Signature bytes (lowercase hex).
    pub signature: String,
}

/// Common shape for contract events and diagnostic events.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Soroban invocation with full args/return payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdrInvocationDto {
    /// Contract address (StrKey) invoked.
    pub contract_id: Option<String>,
    /// Caller StrKey (account or contract).
    pub caller_account: Option<String>,
    /// Invoked function name.
    pub function_name: String,
    /// Function arguments as decoded JSON.
    pub function_args: Vec<serde_json::Value>,
    /// Return value as decoded JSON.
    pub return_value: Option<serde_json::Value>,
    /// Whether this call succeeded.
    pub successful: bool,
    /// Zero-based depth-first index in the invocation tree.
    pub invocation_index: i16,
}

/// Operation raw parameters (XDR-decoded full details).
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E3Response<TxLight> {
    #[serde(flatten)]
    pub light: TxLight,
    pub heavy: Option<E3HeavyFields>,
    pub heavy_fields_status: HeavyFieldsStatus,
}

/// Indicates whether the XDR-sourced fields were loaded successfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeavyFieldsStatus {
    Ok,
    Unavailable,
}
