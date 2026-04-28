//! Database queries for the liquidity-pool endpoints.

use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use super::dto::SharesCursor;

/// Internal row carrying both the wire-visible StrKey and the surrogate
/// `accounts.id` needed for the cursor tie-breaker. The surrogate is
/// stripped before the API response is built.
#[derive(Debug)]
pub struct ParticipantRow {
    /// G-StrKey resolved via JOIN on `accounts`.
    pub account: String,
    /// `accounts.id` BIGINT — used only to encode the next cursor; not
    /// exposed in the response DTO.
    pub account_id_surrogate: i64,
    /// Numeric carried as text to preserve `NUMERIC(28,7)` precision.
    pub shares: String,
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
/// Filters `shares > 0` so withdrawn participants (zero-share rows
/// kept by persist for future-history analytics — see task 0162's
/// emerged decision #2) do not appear in the active-providers view.
pub async fn fetch_participants(
    db: &PgPool,
    pool_id_hex: &str,
    cursor: Option<&SharesCursor>,
    limit: i64,
) -> Result<Vec<ParticipantRow>, sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT a.account_id AS account,
                lpp.account_id AS account_id_surrogate,
                lpp.shares::TEXT AS shares,
                lpp.first_deposit_ledger,
                lpp.last_updated_ledger
           FROM lp_positions lpp
           JOIN accounts a ON a.id = lpp.account_id
          WHERE lpp.pool_id = decode(",
    );
    qb.push_bind(pool_id_hex);
    qb.push(", 'hex') AND lpp.shares > 0");

    if let Some(c) = cursor {
        // Tuple comparison matches the ORDER BY direction: a row strictly
        // before `(cur_shares, cur_acct_id)` in the (shares DESC,
        // account_id DESC) sort is one with a smaller (shares, account_id)
        // tuple.
        qb.push(" AND (lpp.shares, lpp.account_id) < (");
        qb.push_bind(c.shares.clone());
        qb.push("::NUMERIC(28,7), ");
        qb.push_bind(c.account_id);
        qb.push(")");
    }

    qb.push(" ORDER BY lpp.shares DESC, lpp.account_id DESC LIMIT ");
    qb.push_bind(limit);

    let rows = qb.build().fetch_all(db).await?;
    Ok(rows.iter().map(map_participant_row).collect())
}

fn map_participant_row(r: &PgRow) -> ParticipantRow {
    ParticipantRow {
        account: r.get("account"),
        account_id_surrogate: r.get("account_id_surrogate"),
        shares: r.get("shares"),
        first_deposit_ledger: r.get("first_deposit_ledger"),
        last_updated_ledger: r.get("last_updated_ledger"),
    }
}
