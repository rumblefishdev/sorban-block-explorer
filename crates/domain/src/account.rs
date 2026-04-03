//! Account domain type matching the `accounts` PostgreSQL table.

use serde::{Deserialize, Serialize};

/// Account record as stored in PostgreSQL.
///
/// Derived-state entity with ledger-sequence watermarks. Older batches
/// cannot overwrite newer state (guarded by `last_seen_ledger`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// Stellar account address (G... or M..., 56 chars). Primary key.
    pub account_id: String,
    /// Ledger at which the account was first observed (FK to ledgers.sequence).
    pub first_seen_ledger: i64,
    /// Most recent ledger with account activity (FK to ledgers.sequence). Watermark.
    pub last_seen_ledger: i64,
    /// Account transaction sequence number.
    pub sequence_number: i64,
    /// Account balances as JSONB.
    pub balances: serde_json::Value,
    /// Account home domain.
    pub home_domain: Option<String>,
}
