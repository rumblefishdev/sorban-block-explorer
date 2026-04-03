//! NFT domain type matching the `nfts` PostgreSQL table.

use serde::{Deserialize, Serialize};

/// NFT record as stored in PostgreSQL.
///
/// Composite PK: `(contract_id, token_id)`. Derived-state entity with
/// `last_seen_ledger` watermark for concurrency-safe upserts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nft {
    /// Parent contract (FK to soroban_contracts.contract_id).
    pub contract_id: String,
    /// Token identifier within the contract (up to 256 chars).
    pub token_id: String,
    /// Collection name (optional, contracts vary).
    pub collection_name: Option<String>,
    /// Current owner account address.
    pub owner_account: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Media URL (image, video, etc.).
    pub media_url: Option<String>,
    /// Flexible NFT metadata as JSONB.
    pub metadata: Option<serde_json::Value>,
    /// Ledger at which the NFT was minted.
    pub minted_at_ledger: Option<i64>,
    /// Most recent ledger with NFT state change. Watermark.
    pub last_seen_ledger: i64,
}
