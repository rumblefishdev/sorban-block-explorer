//! Request and response DTOs for the liquidity-pool participants endpoint.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Cursor payload for `(shares DESC, account_id DESC)` pagination.
///
/// `shares` is carried as a decimal string (matches `NUMERIC(28,7)`
/// over the wire) so PG comparison stays exact across the fractional
/// component without an f64 round-trip. `account_id` is the surrogate
/// `BIGINT` from `accounts.id` — its direction matches the ORDER BY
/// tie-breaker on equal-shares pages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharesCursor {
    pub shares: String,
    pub account_id: i64,
}

/// One participant row returned by the participants list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ParticipantItem {
    /// Participant account StrKey (G...).
    pub account: String,
    /// Pool-share balance as decimal string (matches `NUMERIC(28,7)`).
    pub shares: String,
    /// Ledger of the first deposit by this account into this pool.
    pub first_deposit_ledger: i64,
    /// Ledger of the most recent change to this position.
    pub last_updated_ledger: i64,
}
