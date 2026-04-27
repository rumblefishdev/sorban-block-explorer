//! Request and response DTOs for the transactions endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// `filter[...]` query parameters for `GET /v1/transactions`.
///
/// `limit` and `cursor` are read by a sibling `Pagination<TsIdCursor>`
/// extractor (see [`crate::common::extractors`]) and documented via the
/// handler's `#[utoipa::path(params(...))]` attribute — they deliberately
/// do not appear here so the two extractors can coexist without fighting
/// over the same fields.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
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

/// DB-sourced light slice for the transaction detail endpoint.
///
/// Composed with `E3HeavyFields` via `merge_e3_response` (from task 0150)
/// into the wrapped E3 response. All XDR-sourced fields (memo, result_code,
/// signatures, events, operations details, envelope_xdr/result_xdr,
/// operation_tree) live in the `heavy` block — see `E3Response`.
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
    /// Operations as known to the DB — type tag + contract_id only. XDR-decoded
    /// per-op detail (function name, raw parameters) lives in
    /// `heavy.operations[]`.
    pub operations: Vec<OperationItem>,
}

/// DB-sourced operation row in `TransactionDetailLight`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OperationItem {
    /// Operation type tag in canonical SCREAMING_SNAKE_CASE
    /// (e.g. `"INVOKE_HOST_FUNCTION"`, `"PAYMENT"`) — produced by
    /// `domain::OperationType`'s `Display` impl.
    #[serde(rename = "type")]
    pub op_type: String,
    /// Contract StrKey (C…) involved in the operation, if applicable.
    pub contract_id: Option<String>,
}
