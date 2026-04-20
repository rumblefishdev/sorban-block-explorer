//! Account-balance domain types matching the `account_balances_current` and
//! `account_balance_history` PostgreSQL tables.
//!
//! Schema: ADR 0027 Part I §17, §18. Native-XLM rows use NULL for both
//! `asset_code` and `issuer_id`; credit-asset rows require both. A CHECK
//! constraint + partial UNIQUE indexes enforce this in the DB.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Current balance per `(account, asset)` (ADR 0027 §17).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBalanceCurrent {
    pub account_id: i64,
    pub asset_type: String,
    /// NULL for `asset_type = 'native'`.
    pub asset_code: Option<String>,
    /// NULL for `asset_type = 'native'`.
    pub issuer_id: Option<i64>,
    /// NUMERIC(28,7) as decimal string.
    pub balance: String,
    pub last_updated_ledger: i64,
}

/// Point-in-time balance row (ADR 0027 §18). Partitioned on `created_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBalanceHistory {
    pub account_id: i64,
    pub ledger_sequence: i64,
    pub asset_type: String,
    pub asset_code: Option<String>,
    pub issuer_id: Option<i64>,
    /// NUMERIC(28,7) as decimal string.
    pub balance: String,
    pub created_at: DateTime<Utc>,
}
