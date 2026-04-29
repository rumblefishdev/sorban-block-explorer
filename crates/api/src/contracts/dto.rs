//! Request and response DTOs for the contracts endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

/// Query parameters shared by `GET /v1/contracts/:contract_id/invocations`
/// and `GET /v1/contracts/:contract_id/events`.
///
/// Note: deliberately *not* `IntoParams`. The `#[utoipa::path]` `params(...)`
/// blocks in `handlers.rs` declare `limit` / `cursor` inline so the generated
/// schema matches every other paginated endpoint (`type: integer`,
/// `type: string`). `IntoParams` would render the `Option<T>` fields as
/// `["T", "null"]`, leaking the in-Rust optionality into the wire format
/// only for these two endpoints.
#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Response types — detail
// ---------------------------------------------------------------------------

/// Aggregate counters over the appearance indexes (ADRs 0033 / 0034).
///
/// `invocation_count` sums `soroban_invocations_appearances.amount` (one per
/// invocation-tree node). `event_count` sums `soroban_events_appearances.amount`
/// (one per non-diagnostic contract event aggregated into the appearance row).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractStats {
    pub invocation_count: i64,
    pub event_count: i64,
}

/// Response for `GET /v1/contracts/:contract_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractDetailResponse {
    pub contract_id: String,
    /// WASM hash hex (64 chars). `null` for SAC / pre-upload contracts.
    pub wasm_hash: Option<String>,
    /// Deployer account StrKey (G…). `null` when the deploy event has not
    /// landed yet (two-pass upsert in the indexer registers bare references
    /// before deployment metadata is observed).
    pub deployer_account: Option<String>,
    pub deployed_at_ledger: Option<i64>,
    /// Explorer-synthetic classification (`token`, `other`, `nft`, `fungible`).
    /// `null` when the deployment metadata has not been observed yet.
    pub contract_type: Option<String>,
    pub is_sac: bool,
    /// Explorer metadata JSON (e.g. `{ "name": "Soroswap DEX" }`).
    pub metadata: Option<serde_json::Value>,
    pub stats: ContractStats,
}

// ---------------------------------------------------------------------------
// Response types — interface
// ---------------------------------------------------------------------------

/// One input parameter on a contract function signature.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InterfaceParam {
    pub name: String,
    /// Soroban / SDK type label (e.g. `"Address"`, `"i128"`).
    #[serde(rename = "type")]
    pub type_name: String,
}

/// One public function on the contract interface.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InterfaceFunction {
    pub name: String,
    pub parameters: Vec<InterfaceParam>,
    /// Return type label. `null` when the spec declares no outputs.
    pub return_type: Option<String>,
}

/// Response for `GET /v1/contracts/:contract_id/interface`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InterfaceResponse {
    pub functions: Vec<InterfaceFunction>,
}

// ---------------------------------------------------------------------------
// Response types — invocations
// ---------------------------------------------------------------------------

/// Single invocation node returned by `GET /v1/contracts/:contract_id/invocations`.
///
/// Per ADR 0034 the DB only carries an appearance index; this row is
/// re-extracted at request time from the public Stellar archive. Depth-first
/// traversal of the auth tree drives the order within a transaction.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InvocationItem {
    /// Parent transaction hash (64-char lowercase hex).
    pub transaction_hash: String,
    /// Account that initiated this call. For root invocations this is the tx
    /// source account; for sub-invocations it is the parent contract's
    /// address. `null` when the appearance row predates a caller observation.
    pub caller_account: Option<String>,
    /// Function name. `null` for contract-creation invocations.
    pub function_name: Option<String>,
    /// ScVal-decoded function arguments (typically a JSON array).
    pub function_args: serde_json::Value,
    /// ScVal-decoded return value (root invocations only; `null` for sub-invocations).
    pub return_value: serde_json::Value,
    /// Whether this invocation succeeded (mirrors the parent transaction).
    pub successful: bool,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Response types — events
// ---------------------------------------------------------------------------

/// Single event returned by `GET /v1/contracts/:contract_id/events`.
///
/// Per ADR 0033 the DB only carries an appearance index; the event payload
/// (type, topics, data) is re-extracted at request time from the public
/// Stellar archive. Diagnostic events are excluded.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventItem {
    /// Parent transaction hash (64-char lowercase hex).
    pub transaction_hash: String,
    /// `"contract"` or `"system"`.
    pub event_type: String,
    /// Decoded topic array.
    pub topics: Vec<serde_json::Value>,
    /// Decoded event data payload.
    pub data: serde_json::Value,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}
