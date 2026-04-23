//! DTOs for XDR-sourced heavy fields, merged with DB-sourced light fields
//! to produce final E3/E14 endpoint responses.
//!
//! Per ADR 0027 Part III + ADR 0029: the DB stores light index columns,
//! heavy payload (memo, signatures, XDR blobs, full event topics+data) lives
//! only in the public Stellar archive. These DTOs represent the heavy slice.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// E3 (`GET /transactions/:hash`) — fields sourced from XDR parse.
///
/// All optional: a tx that parses cleanly exposes every field; on upstream
/// failure the caller substitutes `None` and sets `heavy_fields_status` on
/// the merged response.
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
    /// Contract events — exposed in the light layer as `events`; not serialized here.
    #[serde(skip_serializing)]
    pub contract_events: Vec<XdrEventDto>,
    /// Operations — cross-merged into light `OperationItem` list; not serialized here.
    #[serde(skip_serializing)]
    pub operations: Vec<XdrOperationDto>,
    /// Transaction result code (e.g. `"txSuccess"`, `"txFailed"`).
    /// `None` only when the transaction had a parse error.
    pub result_code: Option<String>,
    /// Nested Soroban invocation tree — exposed in the light layer; not serialized here.
    #[serde(skip_serializing)]
    pub operation_tree: Option<serde_json::Value>,
}

/// E14 (`GET /contracts/:id/events`) — per-event heavy fields.
///
/// The DB holds event identity + `topic0` + transfer prefix; XDR supplies the
/// full topics array and decoded data payload.
#[allow(dead_code)] // used by future E14 events endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E14HeavyEventFields {
    /// Event index within its transaction (matches `soroban_events.event_index`).
    pub event_index: i16,
    /// Transaction hash (hex) this event belongs to — needed for correlation
    /// because a single ledger's events may span many transactions.
    pub transaction_hash: String,
    /// Full topics array as decoded JSON (includes topic0 for self-containment).
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
    /// Event index within the transaction (matches DB `event_index`).
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

/// Merged E14 per-event response: DB light row + optional XDR heavy payload.
#[allow(dead_code)] // used by future E14 events endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E14EventResponse<EventLight> {
    #[serde(flatten)]
    pub light: EventLight,
    pub topics: Option<Vec<serde_json::Value>>,
    pub data: Option<serde_json::Value>,
    pub heavy_fields_status: HeavyFieldsStatus,
}

/// Indicates whether the heavy (XDR-sourced) fields were loaded successfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeavyFieldsStatus {
    Ok,
    Unavailable,
}
