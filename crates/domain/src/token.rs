//! Token domain type matching the `tokens` PostgreSQL table.
//!
//! Schema: ADR 0027 Part I §11. SEP-1 metadata promoted to typed columns
//! per ADR 0023 (`description`, `icon_url`, `home_page`) — legacy
//! `metadata JSONB` is gone.
//!
//! `asset_type ∈ {"native", "classic", "sac", "soroban"}`. Identity by:
//! - classic/SAC: UNIQUE `(asset_code, issuer_id)` (partial)
//! - soroban/SAC: UNIQUE `(contract_id)` (partial)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub id: i32,
    pub asset_type: String,
    pub asset_code: Option<String>,
    pub issuer_id: Option<i64>,
    pub contract_id: Option<String>,
    pub name: Option<String>,
    /// NUMERIC(28,7) as decimal string.
    pub total_supply: Option<String>,
    pub holder_count: Option<i32>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub home_page: Option<String>,
}
