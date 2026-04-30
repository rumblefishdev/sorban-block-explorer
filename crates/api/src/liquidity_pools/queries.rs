//! Database queries for the liquidity-pool endpoints.
//!
//! Shapes pinned to canonical SQL
//! `docs/architecture/database-schema/endpoint-queries/{18,19,20,21,23}_*.sql`.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use super::dto::{ChartDataPoint, PoolListCursor, SharesCursor};

/// Internal row carrying both the wire-visible StrKey and the surrogate
/// `accounts.id` needed for the cursor tie-breaker. The surrogate is
/// stripped before the API response is built.
#[derive(Debug)]
pub(super) struct ParticipantRow {
    /// G-StrKey resolved via JOIN on `accounts`.
    pub account: String,
    /// `accounts.id` BIGINT — used only to encode the next cursor; not
    /// exposed in the response DTO.
    pub account_id_surrogate: i64,
    /// Numeric carried as text to preserve `NUMERIC(28,7)` precision.
    pub shares: String,
    /// `100 * shares / total_pool_shares`, NULL when the pool has no
    /// snapshot in the 7-day freshness window. Already a decimal string
    /// (NUMERIC `::TEXT`) at SELECT time so the API doesn't add an
    /// f64 round-trip.
    pub share_percentage: Option<String>,
    pub first_deposit_ledger: i64,
    pub last_updated_ledger: i64,
}

/// Look up a pool by its hex `pool_id`. Returns `Ok(true)` if the pool
/// exists, `Ok(false)` otherwise. Used to gate 404 vs 200-empty-list on
/// the participants endpoint.
pub async fn pool_exists(db: &PgPool, pool_id_hex: &str) -> Result<bool, sqlx::Error> {
    let row: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM liquidity_pools WHERE pool_id = decode($1, 'hex')")
            .bind(pool_id_hex)
            .fetch_optional(db)
            .await?;
    Ok(row.is_some())
}

/// Fetch up to `limit + 1` participants for a pool ordered by
/// `(shares DESC, account_id DESC)`. The +1 row is the peek used by
/// `common::pagination::finalize_page` to derive `has_more` and the
/// next cursor.
///
/// Filters `lpp.shares > 0` so withdrawn participants (zero-share rows
/// retained by persist for future-history analytics — see task 0162's
/// emerged decision #2) do not appear in the active-providers view.
/// The predicate is intentionally redundant with the partial-index
/// definition (`idx_lpp_shares … WHERE shares > 0`) but kept in the
/// SQL so the query plan remains index-eligible regardless of how
/// future planners weigh it.
///
/// `share_percentage` is computed against the latest snapshot for the
/// pool (within a 7-day freshness window) via a CTE evaluated once per
/// page, joined LATERAL-style to every position row.
pub(super) async fn fetch_participants(
    db: &PgPool,
    pool_id_hex: &str,
    cursor: Option<&SharesCursor>,
    limit: i64,
) -> Result<Vec<ParticipantRow>, sqlx::Error> {
    let (cur_shares, cur_acct): (Option<String>, Option<i64>) = match cursor {
        Some(c) => (Some(c.shares.clone()), Some(c.account_id)),
        None => (None, None),
    };

    // Static query — single plan, NULL-guarded cursor predicate matches
    // the canonical spec in `endpoint-queries/23_*.sql`. The
    // `$3::NUMERIC(28,7) IS NULL` guard short-circuits the keyset on
    // first-page requests.
    let rows = sqlx::query(
        r#"
        WITH latest_snap AS (
            SELECT lps.total_shares
              FROM liquidity_pool_snapshots lps
             WHERE lps.pool_id = decode($1, 'hex')
               AND lps.created_at >= NOW() - INTERVAL '7 days'
             ORDER BY lps.created_at DESC, lps.ledger_sequence DESC
             LIMIT 1
        )
        SELECT
            acc.account_id                  AS account,
            lpp.account_id                  AS account_id_surrogate,
            lpp.shares::TEXT                AS shares,
            CASE
                WHEN snap.total_shares IS NULL OR snap.total_shares = 0 THEN NULL
                ELSE (lpp.shares * 100.0 / snap.total_shares)::TEXT
            END                             AS share_percentage,
            lpp.first_deposit_ledger,
            lpp.last_updated_ledger
          FROM lp_positions lpp
          JOIN accounts acc           ON acc.id = lpp.account_id
          LEFT JOIN latest_snap snap  ON TRUE
         WHERE lpp.pool_id = decode($1, 'hex')
           AND lpp.shares > 0
           AND ($3::numeric IS NULL
                OR (lpp.shares, lpp.account_id) < ($3::numeric, $4::BIGINT))
         ORDER BY lpp.shares DESC, lpp.account_id DESC
         LIMIT $2
        "#,
    )
    .bind(pool_id_hex)
    .bind(limit)
    .bind(cur_shares)
    .bind(cur_acct)
    .fetch_all(db)
    .await?;

    Ok(rows.iter().map(map_participant_row).collect())
}

fn map_participant_row(r: &PgRow) -> ParticipantRow {
    ParticipantRow {
        account: r.get("account"),
        account_id_surrogate: r.get("account_id_surrogate"),
        shares: r.get("shares"),
        share_percentage: r.get("share_percentage"),
        first_deposit_ledger: r.get("first_deposit_ledger"),
        last_updated_ledger: r.get("last_updated_ledger"),
    }
}

// ---------------------------------------------------------------------------
// List / Detail / Transactions / Chart (task 0052)
// ---------------------------------------------------------------------------

/// Canonical column projection shared between list and detail. Matches
/// `18_get_liquidity_pools_list.sql` / `19_get_liquidity_pools_by_id.sql`.
#[derive(Debug, Clone)]
pub struct PoolRow {
    pub pool_id_hex: String,
    pub asset_a_type: i16,
    pub asset_a_type_name: Option<String>,
    pub asset_a_code: Option<String>,
    pub asset_a_issuer: Option<String>,
    pub asset_b_type: i16,
    pub asset_b_type_name: Option<String>,
    pub asset_b_code: Option<String>,
    pub asset_b_issuer: Option<String>,
    pub fee_bps: i32,
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

fn map_pool_row(r: &PgRow) -> PoolRow {
    PoolRow {
        pool_id_hex: r.get("pool_id_hex"),
        asset_a_type: r.get("asset_a_type"),
        asset_a_type_name: r.get("asset_a_type_name"),
        asset_a_code: r.get("asset_a_code"),
        asset_a_issuer: r.get("asset_a_issuer"),
        asset_b_type: r.get("asset_b_type"),
        asset_b_type_name: r.get("asset_b_type_name"),
        asset_b_code: r.get("asset_b_code"),
        asset_b_issuer: r.get("asset_b_issuer"),
        fee_bps: r.get("fee_bps"),
        fee_percent: r.get("fee_percent"),
        created_at_ledger: r.get("created_at_ledger"),
        latest_snapshot_ledger: r.get("latest_snapshot_ledger"),
        reserve_a: r.get("reserve_a"),
        reserve_b: r.get("reserve_b"),
        total_shares: r.get("total_shares"),
        tvl: r.get("tvl"),
        volume: r.get("volume"),
        fee_revenue: r.get("fee_revenue"),
        latest_snapshot_at: r.get("latest_snapshot_at"),
    }
}

pub struct ResolvedPoolListParams {
    pub limit: i64,
    pub cursor: Option<PoolListCursor>,
    pub asset_a_code: Option<String>,
    pub asset_a_issuer: Option<String>,
    pub asset_b_code: Option<String>,
    pub asset_b_issuer: Option<String>,
    /// Decimal string preserving NUMERIC(28,7) precision; passed straight
    /// to `$8::numeric` in the SQL (Postgres parses).
    pub min_tvl: Option<String>,
}

pub async fn fetch_pool_list(
    pool: &PgPool,
    params: &ResolvedPoolListParams,
) -> Result<Vec<PoolRow>, sqlx::Error> {
    // Cursor binds: $2 BIGINT (created_at_ledger), $3 hex string decoded
    // to BYTEA inside SQL via `decode($3, 'hex')`. Doing the decode in SQL
    // (rather than handing sqlx a Vec<u8>) keeps the keyset predicate
    // textually identical to the canonical query and avoids pulling in a
    // hex crate just for this site.
    let (cur_ledger, cur_pool_hex): (Option<i64>, Option<String>) = match &params.cursor {
        Some(c) => (Some(c.created_at_ledger), Some(c.pool_id_hex.clone())),
        None => (None, None),
    };

    let rows = sqlx::query(
        r#"
        WITH issuer_a AS (
            SELECT id FROM accounts WHERE $5::varchar IS NOT NULL AND account_id = $5
        ),
        issuer_b AS (
            SELECT id FROM accounts WHERE $7::varchar IS NOT NULL AND account_id = $7
        )
        SELECT
            encode(lp.pool_id, 'hex')           AS pool_id_hex,
            asset_type_name(lp.asset_a_type)    AS asset_a_type_name,
            lp.asset_a_type                     AS asset_a_type,
            lp.asset_a_code,
            iss_a.account_id                    AS asset_a_issuer,
            asset_type_name(lp.asset_b_type)    AS asset_b_type_name,
            lp.asset_b_type                     AS asset_b_type,
            lp.asset_b_code,
            iss_b.account_id                    AS asset_b_issuer,
            lp.fee_bps,
            (lp.fee_bps::numeric / 100)::text   AS fee_percent,
            lp.created_at_ledger,
            s.ledger_sequence                   AS latest_snapshot_ledger,
            s.reserve_a::text                   AS reserve_a,
            s.reserve_b::text                   AS reserve_b,
            s.total_shares::text                AS total_shares,
            s.tvl::text                         AS tvl,
            s.volume::text                      AS volume,
            s.fee_revenue::text                 AS fee_revenue,
            s.created_at                        AS latest_snapshot_at
        FROM liquidity_pools lp
        LEFT JOIN accounts iss_a ON iss_a.id = lp.asset_a_issuer_id
        LEFT JOIN accounts iss_b ON iss_b.id = lp.asset_b_issuer_id
        LEFT JOIN LATERAL (
            SELECT
                lps.ledger_sequence,
                lps.reserve_a,
                lps.reserve_b,
                lps.total_shares,
                lps.tvl,
                lps.volume,
                lps.fee_revenue,
                lps.created_at
            FROM liquidity_pool_snapshots lps
            WHERE lps.pool_id = lp.pool_id
            ORDER BY lps.created_at DESC, lps.ledger_sequence DESC
            LIMIT 1
        ) s ON TRUE
        WHERE
            ($2::bigint IS NULL
             OR (lp.created_at_ledger, lp.pool_id) < ($2, decode($3::varchar, 'hex')))
            AND ($4::varchar IS NULL OR lp.asset_a_code = $4)
            AND ($5::varchar IS NULL OR lp.asset_a_issuer_id = (SELECT id FROM issuer_a))
            AND ($6::varchar IS NULL OR lp.asset_b_code = $6)
            AND ($7::varchar IS NULL OR lp.asset_b_issuer_id = (SELECT id FROM issuer_b))
            AND ($8::numeric IS NULL OR s.tvl >= $8::numeric)
        ORDER BY lp.created_at_ledger DESC, lp.pool_id DESC
        LIMIT $1
        "#,
    )
    .bind(params.limit)
    .bind(cur_ledger)
    .bind(cur_pool_hex)
    .bind(&params.asset_a_code)
    .bind(&params.asset_a_issuer)
    .bind(&params.asset_b_code)
    .bind(&params.asset_b_issuer)
    .bind(&params.min_tvl)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(map_pool_row).collect())
}

/// `GET /v1/liquidity-pools/:id`. Returns `Ok(None)` for missing pools so
/// the handler can surface 404; database errors propagate as
/// `Err(sqlx::Error)`.
pub async fn fetch_pool_by_id(
    pool: &PgPool,
    pool_id_hex: &str,
) -> Result<Option<PoolRow>, sqlx::Error> {
    let row: Option<PgRow> = sqlx::query(
        r#"
        SELECT
            encode(lp.pool_id, 'hex')          AS pool_id_hex,
            asset_type_name(lp.asset_a_type)   AS asset_a_type_name,
            lp.asset_a_type                    AS asset_a_type,
            lp.asset_a_code,
            iss_a.account_id                   AS asset_a_issuer,
            asset_type_name(lp.asset_b_type)   AS asset_b_type_name,
            lp.asset_b_type                    AS asset_b_type,
            lp.asset_b_code,
            iss_b.account_id                   AS asset_b_issuer,
            lp.fee_bps,
            (lp.fee_bps::numeric / 100)::text  AS fee_percent,
            lp.created_at_ledger,
            s.ledger_sequence                  AS latest_snapshot_ledger,
            s.reserve_a::text                  AS reserve_a,
            s.reserve_b::text                  AS reserve_b,
            s.total_shares::text               AS total_shares,
            s.tvl::text                        AS tvl,
            s.volume::text                     AS volume,
            s.fee_revenue::text                AS fee_revenue,
            s.created_at                       AS latest_snapshot_at
        FROM liquidity_pools lp
        LEFT JOIN accounts iss_a ON iss_a.id = lp.asset_a_issuer_id
        LEFT JOIN accounts iss_b ON iss_b.id = lp.asset_b_issuer_id
        LEFT JOIN LATERAL (
            SELECT
                lps.ledger_sequence,
                lps.reserve_a,
                lps.reserve_b,
                lps.total_shares,
                lps.tvl,
                lps.volume,
                lps.fee_revenue,
                lps.created_at
            FROM liquidity_pool_snapshots lps
            WHERE lps.pool_id = lp.pool_id
            ORDER BY lps.created_at DESC, lps.ledger_sequence DESC
            LIMIT 1
        ) s ON TRUE
        WHERE lp.pool_id = decode($1::varchar, 'hex')
        "#,
    )
    .bind(pool_id_hex)
    .fetch_optional(pool)
    .await?;

    Ok(row.as_ref().map(map_pool_row))
}

/// `GET /v1/liquidity-pools/:id/transactions` row. Mirrors
/// canonical SQL `20_get_liquidity_pools_transactions.sql`.
#[derive(Debug, Clone)]
pub struct PoolTxRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub fee_charged: i64,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn fetch_pool_transactions(
    pool: &PgPool,
    pool_id_hex: &str,
    limit: i64,
    cursor: Option<&crate::common::cursor::TsIdCursor>,
) -> Result<Vec<PoolTxRow>, sqlx::Error> {
    let (cur_ts, cur_id): (Option<DateTime<Utc>>, Option<i64>) = match cursor {
        Some(c) => (Some(c.ts), Some(c.id)),
        None => (None, None),
    };

    let rows = sqlx::query(
        r#"
        WITH matched_ops AS (
            SELECT DISTINCT ON (oa.created_at, oa.transaction_id)
                oa.transaction_id,
                oa.created_at,
                oa.id AS op_appearance_id
            FROM operations_appearances oa
            WHERE oa.pool_id = decode($1::varchar, 'hex')
              AND ($3::timestamptz IS NULL
                   OR (oa.created_at, oa.transaction_id) < ($3, $4))
            ORDER BY oa.created_at DESC, oa.transaction_id DESC, oa.id
            LIMIT $2 * 4
        )
        SELECT
            t.id                    AS id,
            encode(t.hash, 'hex')   AS hash,
            t.ledger_sequence,
            src.account_id          AS source_account,
            t.fee_charged,
            t.successful,
            t.operation_count,
            t.has_soroban,
            COALESCE(ops.operation_types, ARRAY[]::text[]) AS operation_types,
            t.created_at
        FROM matched_ops m
        JOIN transactions t
               ON t.id         = m.transaction_id
              AND t.created_at = m.created_at
        JOIN accounts src ON src.id = t.source_id
        LEFT JOIN LATERAL (
            SELECT array_agg(DISTINCT op_type_name(oa.type)
                             ORDER BY op_type_name(oa.type)) AS operation_types
            FROM operations_appearances oa
            WHERE oa.transaction_id = t.id
              AND oa.created_at     = t.created_at
        ) ops ON TRUE
        ORDER BY t.created_at DESC, t.id DESC
        LIMIT $2
        "#,
    )
    .bind(pool_id_hex)
    .bind(limit)
    .bind(cur_ts)
    .bind(cur_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| PoolTxRow {
            id: r.get("id"),
            hash: r.get("hash"),
            ledger_sequence: r.get("ledger_sequence"),
            source_account: r.get("source_account"),
            fee_charged: r.get("fee_charged"),
            successful: r.get("successful"),
            operation_count: r.get("operation_count"),
            has_soroban: r.get("has_soroban"),
            operation_types: r.get("operation_types"),
            created_at: r.get("created_at"),
        })
        .collect())
}

/// `GET /v1/liquidity-pools/:id/chart`. The interval string is validated
/// by the handler against the `1h | 1d | 1w` allowlist before this is
/// called — the `assert!` below is a defensive second gate.
///
/// `assert!` (not `debug_assert!`) so a release build also panics on
/// allowlist drift instead of silently producing a NULL `bucket` (which
/// would then panic at `r.get::<DateTime<Utc>, _>("bucket")` when sqlx
/// tries to decode NULL into a non-Optional `DateTime`). Cheaper to
/// fail loud on the SQL parameter than at row decode.
pub async fn fetch_pool_chart(
    pool: &PgPool,
    pool_id_hex: &str,
    interval: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<ChartDataPoint>, sqlx::Error> {
    assert!(
        matches!(interval, "1h" | "1d" | "1w"),
        "fetch_pool_chart called with non-allowlisted interval `{interval}` — \
         handler validation drift; expected 1h | 1d | 1w"
    );
    let rows = sqlx::query(
        r#"
        WITH bucket_keyword AS (
            SELECT CASE $2
                WHEN '1h' THEN 'hour'
                WHEN '1d' THEN 'day'
                WHEN '1w' THEN 'week'
            END AS kw
        )
        SELECT
            date_trunc((SELECT kw FROM bucket_keyword), lps.created_at) AS bucket,
            (
                ARRAY_AGG(lps.tvl ORDER BY lps.created_at DESC, lps.ledger_sequence DESC)
            )[1]::text                  AS tvl,
            SUM(lps.volume)::text       AS volume,
            SUM(lps.fee_revenue)::text  AS fee_revenue,
            COUNT(*)                    AS samples_in_bucket
        FROM liquidity_pool_snapshots lps
        WHERE lps.pool_id     = decode($1::varchar, 'hex')
          AND lps.created_at >= $3
          AND lps.created_at <  $4
        GROUP BY date_trunc((SELECT kw FROM bucket_keyword), lps.created_at)
        ORDER BY bucket ASC
        "#,
    )
    .bind(pool_id_hex)
    .bind(interval)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| ChartDataPoint {
            bucket: r.get("bucket"),
            tvl: r.get("tvl"),
            volume: r.get("volume"),
            fee_revenue: r.get("fee_revenue"),
            samples_in_bucket: r.get("samples_in_bucket"),
        })
        .collect())
}
