//! Request and response DTOs for the liquidity-pool endpoints.
//!
//! Participants endpoint (task 0126) and the list/detail/transactions/chart
//! endpoints (tasks 0052) share this module. Wire shapes mirror canonical
//! SQL `endpoint-queries/{18,19,20,21,23}_*.sql`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

// ---------------------------------------------------------------------------
// Participants (task 0126) — UNCHANGED
// ---------------------------------------------------------------------------

/// Cursor payload for `(shares DESC, account_id DESC)` pagination.
///
/// `shares` is carried as a decimal string preserving `NUMERIC(28,7)`
/// precision across the wire so PG comparison stays exact across the
/// fractional component without an f64 round-trip. `account_id` is the
/// surrogate `BIGINT` from `accounts.id` — its direction matches the
/// ORDER BY tie-breaker on equal-shares pages. Cursor stays opaque per
/// ADR 0008; this struct is only deserialized inside the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharesCursor {
    pub shares: String,
    pub account_id: i64,
}

/// One participant row returned by the participants list. Shape pinned to
/// `docs/architecture/database-schema/endpoint-queries/23_get_liquidity_pools_participants.sql`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ParticipantItem {
    /// Participant account StrKey (G...).
    pub account: String,
    /// Pool-share balance carried as a decimal string preserving the
    /// underlying `NUMERIC(28,7)` precision (no f64 round-trip).
    pub shares: String,
    /// Share of the pool, expressed as a decimal-string percentage
    /// (`100 * shares / total_pool_shares`). `None` when the pool has no
    /// snapshot in the freshness window (stale pool); the frontend renders
    /// it as "—" in that case (matches the list-endpoint stale-pool
    /// convention from `18_get_liquidity_pools_list.sql`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_percentage: Option<String>,
    /// Ledger of the first deposit by this account into this pool.
    pub first_deposit_ledger: i64,
    /// Ledger of the most recent change to this position.
    pub last_updated_ledger: i64,
}

// ---------------------------------------------------------------------------
// List / Detail / Transactions / Chart (task 0052)
// ---------------------------------------------------------------------------

/// `filter[...]` query parameters for `GET /v1/liquidity-pools`.
///
/// Asset filters are split per-leg (rather than a single `filter[assets]`
/// composite) to mirror canonical SQL `18_get_liquidity_pools_list.sql`,
/// which takes asset_a_code / asset_a_issuer / asset_b_code / asset_b_issuer
/// as four independent inputs. `limit` / `cursor` are read by a sibling
/// `Pagination<PoolListCursor>` extractor.
#[derive(Debug, Deserialize, IntoParams)]
pub struct PoolListParams {
    #[serde(rename = "filter[asset_a_code]")]
    pub filter_asset_a_code: Option<String>,
    #[serde(rename = "filter[asset_a_issuer]")]
    pub filter_asset_a_issuer: Option<String>,
    #[serde(rename = "filter[asset_b_code]")]
    pub filter_asset_b_code: Option<String>,
    #[serde(rename = "filter[asset_b_issuer]")]
    pub filter_asset_b_issuer: Option<String>,
    /// Minimum TVL threshold as a decimal string (matches the underlying
    /// `NUMERIC(28,7)` column without an f64 round-trip).
    #[serde(rename = "filter[min_tvl]")]
    pub filter_min_tvl: Option<String>,
}

/// One leg of an LP's asset pair. Surfaces both the decoded
/// `asset_type_name` (SQL `asset_type_name()`) and the raw `asset_type`
/// SMALLINT — same contract as `assets/dto::AssetItem`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoolAssetLeg {
    /// `native | classic_credit | sac | soroban`. `null` only on schema drift.
    pub asset_type_name: Option<String>,
    /// Raw SMALLINT (0=native, 1=classic_credit, 2=sac, 3=soroban).
    pub asset_type: i16,
    pub asset_code: Option<String>,
    pub issuer: Option<String>,
}

/// One pool row returned by the list endpoint. Shape pinned to canonical
/// SQL `18_get_liquidity_pools_list.sql`. Pools without a fresh snapshot
/// in the freshness window come back with `null` for every dynamic field
/// (`reserve_a`, `reserve_b`, `total_shares`, `tvl`, `volume`,
/// `fee_revenue`, `latest_snapshot_*`); frontend renders these as "stale".
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoolItem {
    /// 64-char lowercase hex (BYTEA(32) on the wire) per ADR 0024.
    pub pool_id: String,
    pub asset_a: PoolAssetLeg,
    pub asset_b: PoolAssetLeg,
    pub fee_bps: i32,
    /// `fee_bps / 100` as decimal string. Conversion done server-side so
    /// the frontend can render directly (frontend §6.13/§6.14).
    pub fee_percent: String,
    pub created_at_ledger: i64,
    pub latest_snapshot_ledger: Option<i64>,
    pub reserve_a: Option<String>,
    pub reserve_b: Option<String>,
    pub total_shares: Option<String>,
    pub tvl: Option<String>,
    pub volume: Option<String>,
    pub fee_revenue: Option<String>,
    pub latest_snapshot_at: Option<DateTime<Utc>>,
}

/// One row from `/liquidity-pools/:id/transactions`. Shape pinned to
/// canonical SQL `20_get_liquidity_pools_transactions.sql`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoolTransactionItem {
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub fee_charged: i64,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    /// Distinct `op_type_name(...)` labels for every op in the tx, sorted
    /// asc. Frontend §6.14 categorises trade vs LP-mgmt activity from this
    /// list (policy lives client-side, not in SQL).
    pub operation_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Cursor payload for `GET /v1/liquidity-pools` paginated by
/// `(created_at_ledger DESC, pool_id DESC)`. The `pool_id` half travels
/// as 64-char lowercase hex; the SQL decodes it back to BYTEA inside the
/// keyset predicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolListCursor {
    pub created_at_ledger: i64,
    pub pool_id_hex: String,
}

/// Query params for `GET /v1/liquidity-pools/:id/chart`.
///
/// All three params are **optional**. Sensible defaults match the picked
/// interval so a bare request returns a useful chart:
///   - `interval` default: `1d`
///   - `to` default: `now()`
///   - `from` default: `to - <interval-appropriate window>` —
///     `1h → 7 days` (168 buckets), `1d → 90 days` (90 buckets),
///     `1w → 104 weeks` (104 buckets, ≈ 2 years)
///
/// Caller can override any subset. The bucket-count cap (handler-side)
/// rejects ranges that would explode aggregation cost.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ChartParams {
    /// Bucket width: `1h` | `1d` | `1w`. Validated against an allowlist.
    /// Default: `1d`.
    pub interval: Option<String>,
    /// Inclusive lower bound, ISO 8601 / RFC 3339 timestamp.
    /// Default: `to` minus the interval-appropriate window (see struct doc).
    pub from: Option<String>,
    /// Exclusive upper bound, ISO 8601 / RFC 3339 timestamp.
    /// Default: `now()`.
    pub to: Option<String>,
}

/// One row from the chart endpoint. Shape pinned to canonical SQL
/// `21_get_liquidity_pools_chart.sql`. `tvl` is "TVL at close of bucket"
/// (last value); `volume` and `fee_revenue` are SUM (cumulative within
/// the bucket).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChartDataPoint {
    pub bucket: DateTime<Utc>,
    pub tvl: Option<String>,
    pub volume: Option<String>,
    pub fee_revenue: Option<String>,
    pub samples_in_bucket: i64,
}

/// `GET /v1/liquidity-pools/:id/chart` response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChartResponse {
    pub pool_id: String,
    pub interval: String,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub data_points: Vec<ChartDataPoint>,
}
