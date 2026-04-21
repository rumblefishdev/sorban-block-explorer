//! Soroban domain types matching the `soroban_contracts`,
//! `wasm_interface_metadata`, `soroban_events`, and `soroban_invocations`
//! PostgreSQL tables.
//!
//! Schema: ADR 0027 Part I §7, §8, §9, §10.
//! `soroban_contracts.search_vector` is a generated TSVECTOR — DB-only, omitted.
//! Event `topics[1..N]` / `data` and invocation `function_args` / `return_value`
//! live in S3 per ADR 0018.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Contract identity + class + metadata (ADR 0027 §7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanContract {
    pub contract_id: String,
    pub wasm_hash: Option<Vec<u8>>,
    pub wasm_uploaded_at_ledger: Option<i64>,
    pub deployer_id: Option<i64>,
    pub deployed_at_ledger: Option<i64>,
    /// "token" | "dex" | "lending" | "nft" | "other" — NULL until classified.
    pub contract_type: Option<String>,
    pub is_sac: bool,
    pub metadata: Option<serde_json::Value>,
}

/// ABI / WASM metadata keyed by wasm_hash (ADR 0027 §8).
/// Metadata JSONB carries `{ functions: [...], wasm_byte_len: <int> }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmInterfaceMetadata {
    pub wasm_hash: Vec<u8>,
    pub metadata: serde_json::Value,
}

/// Typed transfer-prefix event row (ADR 0027 §9). Partitioned on `created_at`.
/// Full topics/data payload lives in S3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanEvent {
    pub id: i64,
    pub transaction_id: i64,
    pub contract_id: Option<String>,
    pub event_type: String,
    pub topic0: Option<String>,
    pub event_index: i16,
    pub transfer_from_id: Option<i64>,
    pub transfer_to_id: Option<i64>,
    /// NUMERIC(39,0) as decimal string.
    pub transfer_amount: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

/// Caller / function / status row (ADR 0027 §10). Partitioned on `created_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanInvocation {
    pub id: i64,
    pub transaction_id: i64,
    pub contract_id: Option<String>,
    pub caller_id: Option<i64>,
    pub function_name: String,
    pub successful: bool,
    pub invocation_index: i16,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}
