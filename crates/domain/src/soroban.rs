//! Soroban domain types matching the `soroban_contracts`, `soroban_invocations`,
//! and `soroban_events` PostgreSQL tables.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Soroban contract record as stored in PostgreSQL.
///
/// `search_vector` (TSVECTOR generated column) is DB-only and excluded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanContract {
    /// Contract address (C..., 56 chars). Primary key.
    pub contract_id: String,
    /// SHA-256 hash of the WASM bytecode, hex-encoded (64 chars).
    pub wasm_hash: Option<String>,
    /// Account that deployed the contract.
    pub deployer_account: Option<String>,
    /// Ledger at which the contract was deployed (FK to ledgers.sequence).
    pub deployed_at_ledger: Option<i64>,
    /// Explorer-level classification: "token", "dex", "lending", "nft", "other".
    pub contract_type: Option<String>,
    /// Whether this is a Stellar Asset Contract (classic asset wrapped in Soroban).
    pub is_sac: Option<bool>,
    /// Contract metadata JSONB. May contain function signatures.
    pub metadata: Option<serde_json::Value>,
}

/// Soroban invocation record as stored in PostgreSQL.
///
/// Partitioned by `created_at`. Composite PK: `(id, created_at)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanInvocation {
    /// Surrogate primary key (BIGSERIAL).
    pub id: i64,
    /// Parent transaction (FK to transactions.id, CASCADE).
    pub transaction_id: i64,
    /// Invoked contract (FK to soroban_contracts.contract_id).
    pub contract_id: Option<String>,
    /// Account or contract that initiated this call.
    pub caller_account: Option<String>,
    /// Function name invoked.
    pub function_name: String,
    /// ScVal-decoded function arguments as JSONB.
    pub function_args: Option<serde_json::Value>,
    /// ScVal-decoded return value as JSONB.
    pub return_value: Option<serde_json::Value>,
    /// Whether this invocation succeeded.
    pub successful: bool,
    /// Zero-based depth-first index of this node in the invocation tree (dedup key).
    pub invocation_index: i16,
    /// Parent ledger sequence number.
    pub ledger_sequence: i64,
    /// Timestamp for partitioning.
    pub created_at: DateTime<Utc>,
}

/// Soroban event record as stored in PostgreSQL.
///
/// Partitioned by `created_at`. Composite PK: `(id, created_at)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanEvent {
    /// Surrogate primary key (BIGSERIAL).
    pub id: i64,
    /// Parent transaction (FK to transactions.id, CASCADE).
    pub transaction_id: i64,
    /// Contract that emitted the event (FK to soroban_contracts.contract_id).
    pub contract_id: Option<String>,
    /// Event class: "contract", "system", or "diagnostic".
    pub event_type: String,
    /// ScVal-decoded topic values as JSONB array.
    pub topics: serde_json::Value,
    /// ScVal-decoded event data payload as JSONB.
    pub data: serde_json::Value,
    /// Zero-based index of this event within the transaction (dedup key).
    pub event_index: i16,
    /// Parent ledger sequence number.
    pub ledger_sequence: i64,
    /// Timestamp for partitioning.
    pub created_at: DateTime<Utc>,
}
