//! Ledger domain type matching the `ledgers` PostgreSQL table.
//!
//! Schema: ADR 0027 Part I §1. Hash stored as `BYTEA(32)` per ADR 0024;
//! API layer hex-encodes on serialization.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ledger {
    pub sequence: i64,
    pub hash: Vec<u8>,
    pub closed_at: DateTime<Utc>,
    pub protocol_version: i32,
    pub transaction_count: i32,
    pub base_fee: i64,
}
