//! Request and response DTOs for the assets endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// `filter[...]` query parameters for `GET /v1/assets`.
///
/// `limit` / `cursor` are read by a sibling `Pagination<AssetIdCursor>`
/// extractor and are intentionally absent here.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
    #[serde(rename = "filter[type]")]
    pub filter_type: Option<String>,
    /// Substring match against `asset_code`; SQL wraps in `%...%`.
    /// Caller MUST NOT pass `%` / `_` literals.
    #[serde(rename = "filter[code]")]
    pub filter_code: Option<String>,
}

/// Asset row returned by list and detail. Surfaces both the decoded
/// `asset_type_name` (SQL `token_asset_type_name()`) and the raw `asset_type`
/// SMALLINT — canonical SQL `08_get_assets_list.sql` projection.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetItem {
    pub id: i32,
    /// `native | classic_credit | sac | soroban`. `null` only on schema drift.
    pub asset_type_name: Option<String>,
    /// Raw SMALLINT (0=native, 1=classic_credit, 2=sac, 3=soroban).
    pub asset_type: i16,
    pub asset_code: Option<String>,
    pub issuer: Option<String>,
    pub contract_id: Option<String>,
    pub name: Option<String>,
    pub total_supply: Option<String>,
    /// May be `null` / stale until task 0135 ships.
    pub holder_count: Option<i32>,
    pub icon_url: Option<String>,
}

/// Detail response. `description` is populated from the issuer stellar.toml
/// `CURRENCIES[].desc` field; `home_page` is populated from
/// `DOCUMENTATION.ORG_URL` (SEP-1 has no per-currency `home_page` field —
/// the org URL is the closest semantic match and preserves backward
/// compatibility with the previous DB-sourced column). Both default to
/// `null` for native XLM, assets without an issuer, issuers without an
/// on-chain `home_domain`, fetch failures, and stellar.toml files with
/// no matching `[[CURRENCIES]]` entry.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetDetailResponse {
    #[serde(flatten)]
    #[schema(inline)]
    pub item: AssetItem,
    /// `soroban_contracts.deployed_at_ledger` — `null` for classic / native.
    pub deployed_at_ledger: Option<i64>,
    pub description: Option<String>,
    pub home_page: Option<String>,
}

/// Transaction row for `/assets/:id/transactions`. Pure-DB; mirrors
/// canonical SQL `10_get_assets_transactions.sql`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetTransactionItem {
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    pub operation_count: i16,
    pub has_soroban: bool,
    /// Distinct `op_type_name(...)` labels for every op in the tx, sorted asc.
    pub operation_types: Vec<String>,
}
