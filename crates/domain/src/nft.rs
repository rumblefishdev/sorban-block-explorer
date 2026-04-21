//! NFT domain types matching the `nfts` and `nft_ownership` PostgreSQL tables.
//!
//! Schema: ADR 0027 Part I §12, §13. `nfts` gains a surrogate SERIAL PK;
//! identity is still `(contract_id, token_id)` (UNIQUE). Ownership history
//! lives in the partitioned `nft_ownership` table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Current NFT state (ADR 0027 §12).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nft {
    pub id: i32,
    pub contract_id: String,
    pub token_id: String,
    pub collection_name: Option<String>,
    pub name: Option<String>,
    pub media_url: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub minted_at_ledger: Option<i64>,
    pub current_owner_id: Option<i64>,
    pub current_owner_ledger: Option<i64>,
}

/// Per-transfer ownership record (ADR 0027 §13). Partitioned on `created_at`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NftOwnership {
    pub nft_id: i32,
    pub transaction_id: i64,
    pub owner_id: Option<i64>,
    /// "mint" | "transfer" | "burn".
    pub event_type: String,
    pub ledger_sequence: i64,
    pub event_order: i16,
    pub created_at: DateTime<Utc>,
}
