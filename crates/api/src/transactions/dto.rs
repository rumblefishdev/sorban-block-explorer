//! Request and response DTOs for the transactions endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// `filter[...]` query parameters for `GET /v1/transactions`.
///
/// `limit` and `cursor` are read by a sibling `Pagination<TsIdCursor>`
/// extractor and documented via the handler's `#[utoipa::path(params(...))]`
/// attribute.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
    /// Filter by source account StrKey (G…).
    #[serde(rename = "filter[source_account]")]
    pub filter_source_account: Option<String>,
    /// Filter by contract StrKey (C…) — matches root op, nested call, or event emission.
    #[serde(rename = "filter[contract_id]")]
    pub filter_contract_id: Option<String>,
    /// Filter by operation type (e.g. `INVOKE_HOST_FUNCTION`).
    #[serde(rename = "filter[operation_type]")]
    pub filter_operation_type: Option<String>,
}

/// Slim transaction row returned in the list endpoint.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionListItem {
    /// Transaction hash (64-char lowercase hex).
    pub hash: String,
    pub ledger_sequence: i64,
    /// 1-based position of this transaction within its ledger.
    pub application_order: i16,
    pub source_account: String,
    /// Fee charged in stroops.
    pub fee_charged: i64,
    /// Inner-transaction hash (64-char hex) for fee-bump envelopes, `null` otherwise.
    pub inner_tx_hash: Option<String>,
    pub successful: bool,
    pub operation_count: i16,
    /// `true` when the transaction touched at least one Soroban contract
    /// (root invocation, nested call, or event emission).
    pub has_soroban: bool,
    /// All distinct operation type names in the transaction
    /// (e.g. `["INVOKE_HOST_FUNCTION", "PAYMENT"]`).
    pub operation_types: Vec<String>,
    /// All C-StrKeys touched anywhere in the transaction.
    pub contract_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    /// Memo type: `"none"`, `"text"`, `"id"`, `"hash"`, or `"return"`.
    /// `null` when the XDR fetch failed.
    pub memo_type: Option<String>,
    /// Memo value. `null` when no memo or XDR fetch failed.
    pub memo: Option<String>,
}

/// DB-sourced light slice for the transaction detail endpoint.
///
/// Composed with `E3HeavyFields` via `merge_e3_response` (task 0150). All
/// XDR-sourced fields (memo, result_code, signatures, events, operation
/// details, envelope_xdr/result_xdr, operation_tree) live in `heavy`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionDetailLight {
    /// Transaction hash (64-char lowercase hex).
    pub hash: String,
    pub ledger_sequence: i64,
    /// 1-based position of this transaction within its ledger.
    pub application_order: i16,
    pub source_account: String,
    /// Fee charged in stroops.
    pub fee_charged: i64,
    /// Inner-transaction hash (64-char hex) for fee-bump envelopes, `null` otherwise.
    pub inner_tx_hash: Option<String>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub created_at: DateTime<Utc>,
    /// `true` when the XDR parser encountered an error for this transaction.
    pub parse_error: bool,
    pub operations: Vec<OperationItem>,
    /// Accounts touched by this transaction. Populated only when
    /// `heavy_fields_status = "unavailable"`; otherwise `[]` and consumers
    /// should rely on the heavy block.
    pub participants: Vec<String>,
    /// Soroban event appearance index rows. Same fallback semantics as
    /// `participants`. Full topics + data live in `heavy.contract_events`.
    pub soroban_events: Vec<EventAppearanceItem>,
    /// Soroban invocation appearance index rows. Same fallback semantics
    /// as `participants`. Full call hierarchy lives in `heavy.operation_tree`.
    pub soroban_invocations: Vec<InvocationAppearanceItem>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventAppearanceItem {
    pub contract_id: String,
    pub ledger_sequence: i64,
    pub amount: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvocationAppearanceItem {
    pub contract_id: String,
    /// Root caller G-StrKey. Per ADR 0034 nested-call hierarchy is XDR-only.
    pub caller_account: Option<String>,
    pub ledger_sequence: i64,
    pub amount: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OperationItem {
    /// Global BIGSERIAL `operations_appearances.id`; result-set order
    /// (`ORDER BY oa.id`) is the operation's within-tx application order.
    pub appearance_id: i64,
    /// Operation type tag in canonical SCREAMING_SNAKE_CASE
    /// (e.g. `"INVOKE_HOST_FUNCTION"`).
    pub type_name: String,
    /// Raw `OperationType` SMALLINT (ADR 0031).
    #[serde(rename = "type")]
    pub op_type: i16,
    pub source_account: Option<String>,
    pub destination_account: Option<String>,
    pub contract_id: Option<String>,
    /// Asset code (≤12 chars) for classic asset operations.
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    /// Hex-encoded liquidity pool ID.
    pub pool_id: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}
