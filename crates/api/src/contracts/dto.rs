//! Request and response DTOs for the contracts endpoints.
//! Wire shapes mirror canonical SQL `endpoint-queries/{11..14}_*.sql`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractStats {
    pub recent_invocations: i64,
    pub recent_unique_callers: i64,
    /// Echoed window label (e.g. `"7 days"`) so the UI can label "last N days".
    pub stats_window: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ContractDetailResponse {
    pub contract_id: String,
    pub wasm_hash: Option<String>,
    pub wasm_uploaded_at_ledger: Option<i64>,
    pub deployer: Option<String>,
    pub deployed_at_ledger: Option<i64>,
    pub contract_type_name: Option<String>,
    pub contract_type: Option<i16>,
    pub is_sac: bool,
    // `metadata` field removed per ADR 0041 / task 0156. The
    // underlying `soroban_contracts.metadata JSONB` was replaced by
    // typed `name VARCHAR(256)`; the field was always `{}` in
    // practice and carried no information for the detail view.
    // Frontend already handled the empty-object case as "no
    // metadata"; absent field has the same effect.
    pub stats: ContractStats,
}

/// `interface_metadata` is `null` for SAC / pre-upload / stub rows;
/// stubs (task 0153) are filtered at the SQL layer so they don't leak.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InterfaceResponse {
    pub contract_id: String,
    pub wasm_hash: Option<String>,
    pub interface_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InvocationItem {
    pub transaction_hash: String,
    pub ledger_sequence: i64,
    pub caller_account: Option<String>,
    /// Folded invocation-tree node count for this appearance.
    pub amount: i32,
    pub created_at: DateTime<Utc>,
    pub successful: bool,
}

/// One row per event — an appearance with `amount > 1` expands to that
/// many rows (per-tx fields repeated, per-event fields unique).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventItem {
    pub transaction_hash: String,
    pub ledger_sequence: i64,
    pub transaction_id: i64,
    pub successful: bool,
    pub amount: i64,
    pub created_at: DateTime<Utc>,
    pub event_type: String,
    pub topics: Vec<serde_json::Value>,
    pub data: serde_json::Value,
}
