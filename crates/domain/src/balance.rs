//! Account-balance domain types matching the `account_balances_current`
//! PostgreSQL table.
//!
//! Schema: ADR 0027 Part I §17 (ADR 0035 dropped §18 `account_balance_history`).
//! Native-XLM rows use NULL for both `asset_code` and `issuer_id`; credit-asset
//! rows require both. A CHECK constraint + partial UNIQUE indexes enforce this
//! in the DB.

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
