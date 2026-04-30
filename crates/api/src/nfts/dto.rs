//! Request and response DTOs for the NFT endpoints.
//! Wire shapes mirror canonical SQL `endpoint-queries/{15,16,17}_*.sql`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// `filter[...]` query parameters for `GET /v1/nfts`.
///
/// `limit` / `cursor` are read by a sibling `Pagination<NftIdCursor>`
/// extractor and are intentionally absent here.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
    /// Exact match against `nfts.collection_name` (btree
    /// `idx_nfts_collection`). Trigram support is task 0132.
    #[serde(rename = "filter[collection]")]
    pub filter_collection: Option<String>,
    /// Contract C-StrKey; resolved to `soroban_contracts.id` server-side.
    #[serde(rename = "filter[contract_id]")]
    pub filter_contract_id: Option<String>,
    /// Substring match against `nfts.name` via the `idx_nfts_name_trgm`
    /// GIN index. SQL wraps the value in `%...%`; caller MUST NOT pass
    /// `%` / `_` literals.
    #[serde(rename = "filter[name]")]
    pub filter_name: Option<String>,
}

/// One NFT row returned by the list endpoint. Shape pinned to canonical
/// SQL `15_get_nfts_list.sql`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NftItem {
    pub id: i32,
    /// Contract C-StrKey resolved via `soroban_contracts` join.
    pub contract_id: String,
    pub token_id: String,
    pub collection_name: Option<String>,
    pub name: Option<String>,
    pub media_url: Option<String>,
    /// Verbatim JSONB at mint time. Shape is contract-defined (no canonical
    /// schema yet) — frontend renders defensively.
    pub metadata: Option<serde_json::Value>,
    pub minted_at_ledger: Option<i64>,
    /// Current owner G-StrKey, or `null` for burned NFTs (ADR 0037 §13).
    pub owner_account: Option<String>,
    /// Most recent ledger where ownership state changed
    /// (`nfts.current_owner_ledger`).
    pub last_seen_ledger: Option<i64>,
}

/// One row of NFT transfer history. Shape pinned to canonical SQL
/// `17_get_nfts_transfers.sql`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NftTransferItem {
    pub transaction_hash: String,
    pub ledger_sequence: i64,
    /// `mint | transfer | burn` — pre-decoded via `nft_event_type_name(...)`.
    pub event_type_name: Option<String>,
    /// Raw NftEventType discriminant (ADR 0031).
    pub event_type: i16,
    /// Previous-owner G-StrKey reconstructed via `LEAD(owner_id)` over the
    /// per-NFT ownership timeline (DESC window — older event sits at the
    /// FOLLOWING window position). `null` on the mint row.
    ///
    /// **Pagination caveat:** within a single page LEAD works correctly.
    /// At page boundaries the *last* row's `from_account` is reset to
    /// `null` because the row below it (the next-page first row) isn't in
    /// the current result set; clients can stitch by remembering the next
    /// page's first `to_account`.
    pub from_account: Option<String>,
    /// New owner G-StrKey. `null` on burn.
    pub to_account: Option<String>,
    pub created_at: DateTime<Utc>,
    pub event_order: i16,
}

/// Cursor payload for `GET /v1/nfts`. The `nfts` table has a SERIAL
/// surrogate PK and the canonical SQL orders by `id DESC` — `TsIdCursor`
/// does not fit because there's no `created_at` column on `nfts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NftIdCursor {
    pub id: i32,
}

/// Cursor payload for `GET /v1/nfts/:id/transfers`. The natural keyset
/// is the `nft_ownership` PK `(nft_id, created_at, ledger_sequence,
/// event_order)`; `nft_id` is a path parameter so only the trailing
/// three components live in the cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NftTransferCursor {
    pub created_at: DateTime<Utc>,
    pub ledger_sequence: i64,
    pub event_order: i16,
}
