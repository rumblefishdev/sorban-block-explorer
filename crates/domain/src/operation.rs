//! Operation appearance row matching the `operations_appearances` PostgreSQL
//! table (task 0163).
//!
//! Schema: ADR 0027 Part I §5 + task 0163. Partitioned on `created_at`,
//! composite PK `(id, created_at)`. Appearance index — one row per distinct
//! operation identity in a transaction, `amount` counts collapsed duplicates.
//! Per-op detail (transfer amount, application order, memo, claimants,
//! function args, …) lives in XDR archived in S3 per ADR 0018 and is
//! re-materialised by the API via `xdr_parser::extract_operations`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::enums::OperationType;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationAppearance {
    pub id: i64,
    pub transaction_id: i64,
    /// DDL column name is `type` (reserved in Rust). Stored as SMALLINT per
    /// ADR 0031; `OperationType` derives `sqlx::Type` under the `sqlx`
    /// feature for round-tripping through the DB.
    #[serde(rename = "type")]
    pub op_type: OperationType,
    /// Muxed source override; inherited from transaction when NULL.
    pub source_id: Option<i64>,
    pub destination_id: Option<i64>,
    pub contract_id: Option<i64>,
    pub asset_code: Option<String>,
    pub asset_issuer_id: Option<i64>,
    /// Classic liquidity-pool id (32 B BYTEA).
    pub pool_id: Option<Vec<u8>>,
    /// Count of operations of this identity collapsed into the row.
    pub amount: i64,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}
