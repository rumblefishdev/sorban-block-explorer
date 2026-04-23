//! Request and response DTOs for the transactions endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Query parameters for `GET /v1/transactions`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
    /// Items per page (1–100, default 20).
    pub limit: Option<u32>,
    /// Opaque pagination cursor from a previous response.
    pub cursor: Option<String>,
    /// Filter by source account StrKey (G…).
    #[serde(rename = "filter[source_account]")]
    pub filter_source_account: Option<String>,
    /// Filter by contract StrKey (C…) that appears in an operation.
    #[serde(rename = "filter[contract_id]")]
    pub filter_contract_id: Option<String>,
    /// Filter by operation type (e.g. `INVOKE_HOST_FUNCTION`).
    #[serde(rename = "filter[operation_type]")]
    pub filter_operation_type: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Slim transaction row returned in the list endpoint.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionListItem {
    /// Transaction hash (64-char lowercase hex).
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    /// Fee charged in stroops.
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    pub operation_count: i16,
    /// Memo type: `"none"`, `"text"`, `"id"`, `"hash"`, or `"return"`.
    /// `null` when the XDR fetch failed (graceful degradation).
    pub memo_type: Option<String>,
    /// Memo value. `null` when no memo or XDR fetch failed.
    pub memo: Option<String>,
}

/// Query parameters for `GET /v1/transactions/:hash`.
#[derive(Debug, Deserialize, IntoParams)]
pub struct DetailParams {
    /// Set to `advanced` to fetch XDR heavy fields from the public Stellar archive.
    pub view: Option<String>,
}

/// Transaction detail response per 0046 spec.
///
/// Both the normal and advanced views share this shape. The XDR-sourced
/// fields (`memo_type`, `memo`, `result_code`, `operation_tree`, `events`,
/// per-op `function_name`) are always populated when the public-archive
/// fetch succeeds and degrade gracefully to `null` / empty on fetch
/// failure. The advanced-only fields (`envelope_xdr`, `result_xdr`, per-op
/// `raw_parameters`) are absent from the JSON in the normal view.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionDetailLight {
    /// Transaction hash (64-char lowercase hex).
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    /// Fee charged in stroops.
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    /// `true` when the XDR parser encountered an error for this transaction.
    pub parse_error: bool,
    /// Memo type: `"none"`, `"text"`, `"id"`, `"hash"`, or `"return"`.
    /// `null` when the public-archive fetch failed.
    pub memo_type: Option<String>,
    /// Memo value. `null` when no memo or fetch failed.
    pub memo: Option<String>,
    /// Transaction result code (e.g. `"txSuccess"`, `"txFailed"`).
    /// `null` when fetch failed or when `parse_error == true`.
    pub result_code: Option<String>,
    /// Operations within the transaction. `function_name` is populated when
    /// XDR is available; `raw_parameters` is populated only in advanced view.
    pub operations: Vec<OperationItem>,
    /// Nested Soroban invocation tree. `null` when fetch failed.
    pub operation_tree: Option<serde_json::Value>,
    /// Soroban contract events with full topics + data. Empty when fetch failed.
    pub events: Vec<EventItem>,
    /// Base64-encoded `TransactionEnvelope`. Advanced view only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope_xdr: Option<String>,
    /// Base64-encoded `TransactionResult`. Advanced view only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_xdr: Option<String>,
}

/// One operation within a transaction detail response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OperationItem {
    /// Operation type tag (e.g. `"invoke_host_function"`, `"payment"`).
    #[serde(rename = "type")]
    pub op_type: String,
    /// Contract StrKey (C…) involved in the operation, if applicable.
    pub contract_id: Option<String>,
    /// Invoked function name. Populated when XDR is available; `null` for
    /// non-Soroban ops or when the public-archive fetch failed.
    pub function_name: Option<String>,
    /// Full XDR-decoded operation details. Advanced view only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_parameters: Option<serde_json::Value>,
}

/// One Soroban contract event within a transaction detail response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventItem {
    /// Event type: `"contract"`, `"system"`, or `"diagnostic"`.
    pub event_type: String,
    /// Contract StrKey (C…) that emitted the event.
    pub contract_id: Option<String>,
    /// Full decoded topics array.
    pub topics: Vec<serde_json::Value>,
    /// Decoded event data.
    pub data: serde_json::Value,
}
