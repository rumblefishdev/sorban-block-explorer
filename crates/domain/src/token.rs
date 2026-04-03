//! Token domain type matching the `tokens` PostgreSQL table.

use serde::{Deserialize, Serialize};

/// Token record as stored in PostgreSQL.
///
/// Unpartitioned derived-state table. Identity depends on `asset_type`:
/// - Classic/SAC: `UNIQUE(asset_code, issuer_address)`
/// - Soroban/SAC: `UNIQUE(contract_id)`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    /// Surrogate primary key (SERIAL).
    pub id: i32,
    /// Token classification: "classic", "sac", or "soroban".
    pub asset_type: String,
    /// Classic asset code (up to 12 chars). Null for pure Soroban tokens.
    pub asset_code: Option<String>,
    /// Classic asset issuer (G... address). Null for pure Soroban tokens.
    pub issuer_address: Option<String>,
    /// Soroban contract address (FK to soroban_contracts). Null for classic-only.
    pub contract_id: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Total supply as string (NUMERIC precision).
    pub total_supply: Option<String>,
    /// Number of accounts holding this token.
    pub holder_count: Option<i32>,
    /// Flexible token metadata as JSONB.
    pub metadata: Option<serde_json::Value>,
}
