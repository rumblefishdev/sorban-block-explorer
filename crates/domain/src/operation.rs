//! Operation domain type matching the `operations` PostgreSQL table.
//!
//! Schema: ADR 0027 Part I §5. Partitioned on `created_at`, composite PK
//! `(id, created_at)`. Details JSONB is gone — frequently queried fields
//! are promoted to typed columns; raw op params live in S3 per ADR 0018.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: i64,
    pub transaction_id: i64,
    pub application_order: i16,
    /// DDL column name is `type` (reserved in Rust).
    #[serde(rename = "type")]
    pub op_type: String,
    /// Muxed source override; inherited from transaction when NULL.
    pub source_id: Option<i64>,
    pub destination_id: Option<i64>,
    /// Soroban contract (StrKey `C…`) when op targets a contract.
    pub contract_id: Option<String>,
    pub asset_code: Option<String>,
    pub asset_issuer_id: Option<i64>,
    /// Classic liquidity-pool id (32 B BYTEA).
    pub pool_id: Option<Vec<u8>>,
    /// Payment / path-payment / transfer amount (NUMERIC(28,7) as string).
    pub transfer_amount: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}
