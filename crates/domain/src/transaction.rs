//! Transaction-family domain types matching the `transactions`,
//! `transaction_hash_index`, and `transaction_participants` PostgreSQL tables.
//!
//! Schema: ADR 0027 Part I §3, §4, §6.
//! Heavy fields (memo, signatures, XDR, diagnostic events, operation tree)
//! live in S3 per ADR 0011/0018 — they are not in these structs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Transaction core row (ADR 0027 §3). Partitioned monthly on `created_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub id: i64,
    pub hash: Vec<u8>,
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub source_id: i64,
    pub fee_charged: i64,
    pub inner_tx_hash: Option<Vec<u8>>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub parse_error: bool,
    pub created_at: DateTime<Utc>,
}

/// Hash → (ledger, created_at) lookup (ADR 0027 §4).
/// Unpartitioned, feeds `/transactions/:hash` routing preflight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionHashIndex {
    pub hash: Vec<u8>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

/// `(account, transaction)` edge (ADR 0027 §6). Partitioned on `created_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionParticipant {
    pub transaction_id: i64,
    pub account_id: i64,
    pub created_at: DateTime<Utc>,
}
