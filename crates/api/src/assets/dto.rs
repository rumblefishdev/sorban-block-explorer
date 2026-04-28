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
    #[serde(rename = "filter[code]")]
    pub filter_code: Option<String>,
}

/// Asset row returned by list and detail. SEP-1 fields (`description`,
/// `home_page`) live in S3 per ADR 0037 §342 — see [`AssetDetailResponse`].
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetItem {
    pub id: i32,
    pub asset_type: String,
    pub asset_code: Option<String>,
    pub issuer_address: Option<String>,
    pub contract_id: Option<String>,
    pub name: Option<String>,
    pub total_supply: Option<String>,
    pub holder_count: Option<i32>,
    pub icon_url: Option<String>,
}

/// Detail response. `description` / `home_page` are always `null` until
/// the S3 hydration follow-up (task 0164) ships.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetDetailResponse {
    #[serde(flatten)]
    #[schema(inline)]
    pub item: AssetItem,
    pub description: Option<String>,
    pub home_page: Option<String>,
}

/// Transaction row for `/assets/:id/transactions`. No memo enrichment —
/// pure-DB on this endpoint avoids a per-page S3 fetch.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetTransactionItem {
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    pub operation_count: i16,
}
