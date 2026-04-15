//! Transaction domain type matching the `transactions` PostgreSQL table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Transaction record as stored in PostgreSQL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    /// Surrogate primary key (BIGSERIAL).
    pub id: i64,
    /// SHA-256 hash of the TransactionEnvelope, hex-encoded (64 chars).
    pub hash: String,
    /// Parent ledger sequence number.
    pub ledger_sequence: i64,
    /// Transaction source account (G... or M... address).
    pub source_account: String,
    /// Actual fee charged in stroops.
    pub fee_charged: i64,
    /// Whether the transaction succeeded.
    pub successful: bool,
    /// Transaction result code string.
    pub result_code: Option<String>,
    /// Memo type: "text", "id", "hash", "return", or None.
    pub memo_type: Option<String>,
    /// Memo value.
    pub memo: Option<String>,
    /// Timestamp derived from ledger close time.
    pub created_at: DateTime<Utc>,
    /// True if XDR parsing failed.
    pub parse_error: Option<bool>,
}
