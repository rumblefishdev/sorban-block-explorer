//! Asset domain type matching the `assets` PostgreSQL table.
//!
//! Schema: ADR 0027 Part I §11 + ADR 0036 rename. Current shape recorded in
//! ADR 0037 (task 0164): `icon_url` is the only SEP-1 field on the DB row
//! and serves list-level thumbnail rendering. Asset-detail metadata
//! (`description`, `home_page`) lives per-entity in S3 at
//! `s3://<bucket>/assets/{id}.json`. Legacy `metadata JSONB` is also gone.
//!
//! `asset_type ∈ {"native", "classic_credit", "sac", "soroban"}`. Identity by:
//! - classic_credit/SAC: UNIQUE `(asset_code, issuer_id)` (partial)
//! - soroban/SAC: UNIQUE `(contract_id)` (partial)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: i32,
    pub asset_type: String,
    pub asset_code: Option<String>,
    pub issuer_id: Option<i64>,
    pub contract_id: Option<String>,
    pub name: Option<String>,
    /// NUMERIC(28,7) as decimal string.
    pub total_supply: Option<String>,
    pub holder_count: Option<i32>,
    pub icon_url: Option<String>,
}
