//! Account domain type matching the `accounts` PostgreSQL table.
//!
//! Schema: ADR 0027 Part I §2 (surrogate PK — ADR 0026).
//! Balances moved to `account_balances_current` (see `crate::balance`).

use serde::{Deserialize, Serialize};

/// Account record as stored in PostgreSQL.
///
/// `id` is the surrogate PK used for all FK references across the schema.
/// `account_id` is the StrKey form (`G…`, 56 chars) — UNIQUE, used for
/// display and StrKey→id resolution at route-param intake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: i64,
    pub account_id: String,
    pub first_seen_ledger: i64,
    pub last_seen_ledger: i64,
    pub sequence_number: i64,
    pub home_domain: Option<String>,
}
