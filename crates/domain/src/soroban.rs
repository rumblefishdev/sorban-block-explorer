//! Soroban domain types matching the `soroban_contracts` and
//! `wasm_interface_metadata` PostgreSQL tables.
//!
//! Schema: ADR 0027 Part I §7, §8.
//! `soroban_contracts.search_vector` is a generated TSVECTOR — DB-only, omitted.
//! Event detail (type, topics, data, transfer triple) lives exclusively on the
//! public Stellar archive per ADR 0033; the `soroban_events_appearances` table
//! is a pure index queried directly by the API without a domain mirror type.
//! Invocation per-node detail (function name, args, return value, successful,
//! depth) lives on the public archive per ADR 0034;
//! `soroban_invocations_appearances` is queried directly by the API without a
//! domain mirror type (same pattern as events).

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
