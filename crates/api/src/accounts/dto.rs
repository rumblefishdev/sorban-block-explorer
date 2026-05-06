//! Wire shapes mirror canonical SQL `endpoint-queries/{06,07}_*.sql`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Native rows have `null` `asset_code` / `asset_issuer`; credit rows have both.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountBalance {
    /// `native` | `credit_alphanum4` | `credit_alphanum12`.
    pub asset_type_name: Option<String>,
    /// Raw SMALLINT — stable across label renames.
    #[serde(rename = "type")]
    pub asset_type: i16,
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    /// `NUMERIC(28,7)` as fixed-precision string (preserves trailing zeros).
    pub balance: String,
    pub last_updated_ledger: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountDetailResponse {
    pub account_id: String,
    pub sequence_number: i64,
    pub balances: Vec<AccountBalance>,
    pub home_domain: Option<String>,
    pub first_seen_ledger: i64,
    pub last_seen_ledger: i64,
}

/// Slim — `inner_tx_hash` / `contract_ids[]` live on `/v1/transactions` only.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountTransactionItem {
    /// 64-char lowercase hex.
    pub hash: String,
    pub ledger_sequence: i64,
    /// 1-based position in ledger.
    pub application_order: i16,
    pub source_account: String,
    /// Stroops.
    pub fee_charged: i64,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}
