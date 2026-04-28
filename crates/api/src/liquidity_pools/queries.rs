//! Database queries for the liquidity-pool endpoints.
//!
//! Shapes pinned to
//! `docs/architecture/database-schema/endpoint-queries/23_get_liquidity_pools_participants.sql`.

use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use super::dto::SharesCursor;

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
