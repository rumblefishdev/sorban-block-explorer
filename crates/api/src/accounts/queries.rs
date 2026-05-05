//! Mirrors canonical SQL `endpoint-queries/{06,07}_*.sql`.
//! `transaction_participants` includes source, so no UNION with `source_id`.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use crate::common::cursor::TsIdCursor;

#[derive(Debug)]
pub struct AccountHeaderRow {
    /// Surrogate id, threaded into balances query — never on wire.
    pub id: i64,
    pub account_id: String,
    pub first_seen_ledger: i64,
    pub last_seen_ledger: i64,
    pub sequence_number: i64,
    pub home_domain: Option<String>,
}

#[derive(Debug)]
pub struct AccountBalanceRow {
    pub asset_type_name: Option<String>,
    pub asset_type: i16,
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    pub balance: String,
    pub last_updated_ledger: i64,
}

#[derive(Debug)]
pub struct AccountTxRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub source_account: String,
    pub fee_charged: i64,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Detail header query — canonical 06 Statement A
// ---------------------------------------------------------------------------

/// `Ok(None)` → handler returns 404.
pub async fn fetch_account(
    pool: &PgPool,
    account_strkey: &str,
) -> Result<Option<AccountHeaderRow>, sqlx::Error> {
    let raw: Option<PgRow> = sqlx::query(
        "SELECT \
            a.id, \
            a.account_id, \
            a.first_seen_ledger, \
            a.last_seen_ledger, \
            a.sequence_number, \
            a.home_domain \
         FROM accounts a \
         WHERE a.account_id = $1",
    )
    .bind(account_strkey)
    .fetch_optional(pool)
    .await?;

    Ok(raw.map(|r| AccountHeaderRow {
        id: r.get("id"),
        account_id: r.get("account_id"),
        first_seen_ledger: r.get("first_seen_ledger"),
        last_seen_ledger: r.get("last_seen_ledger"),
        sequence_number: r.get("sequence_number"),
        home_domain: r.get("home_domain"),
    }))
}

// ---------------------------------------------------------------------------
// Detail balances query — canonical 06 Statement B
// ---------------------------------------------------------------------------

/// `account_id` is the surrogate from [`fetch_account`].
pub async fn fetch_balances(
    pool: &PgPool,
    account_id: i64,
) -> Result<Vec<AccountBalanceRow>, sqlx::Error> {
    let raw: Vec<PgRow> = sqlx::query(
        "SELECT \
            asset_type_name(abc.asset_type) AS asset_type_name, \
            abc.asset_type                  AS asset_type, \
            abc.asset_code, \
            iss.account_id                  AS asset_issuer, \
            abc.balance::text               AS balance, \
            abc.last_updated_ledger \
         FROM account_balances_current abc \
         LEFT JOIN accounts iss ON iss.id = abc.issuer_id \
         WHERE abc.account_id = $1 \
         ORDER BY abc.asset_type, abc.asset_code, iss.account_id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;

    Ok(raw
        .iter()
        .map(|r| AccountBalanceRow {
            asset_type_name: r.get("asset_type_name"),
            asset_type: r.get("asset_type"),
            asset_code: r.get("asset_code"),
            asset_issuer: r.get("asset_issuer"),
            balance: r.get("balance"),
            last_updated_ledger: r.get("last_updated_ledger"),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Transactions query — canonical 07
// ---------------------------------------------------------------------------

/// CTE inlines StrKey → BIGINT so the planner walks the
/// `transaction_participants` PK keyset. Caller passes `limit + 1`.
pub async fn fetch_transactions(
    pool: &PgPool,
    account_strkey: &str,
    limit: i64,
    cursor: Option<&TsIdCursor>,
) -> Result<Vec<AccountTxRow>, sqlx::Error> {
    let cursor_ts = cursor.map(|c| c.ts);
    let cursor_id = cursor.map(|c| c.id);

    let raw: Vec<PgRow> = sqlx::query(
        "WITH acc AS ( \
             SELECT id FROM accounts WHERE account_id = $1 \
         ) \
         SELECT \
             t.id, \
             encode(t.hash, 'hex')          AS hash, \
             t.ledger_sequence, \
             t.application_order, \
             src.account_id                 AS source_account, \
             t.fee_charged, \
             t.successful, \
             t.operation_count, \
             t.has_soroban, \
             COALESCE(ops.operation_types, ARRAY[]::text[]) AS operation_types, \
             t.created_at \
         FROM acc \
         JOIN transaction_participants tp ON tp.account_id = acc.id \
         JOIN transactions t \
                ON t.id         = tp.transaction_id \
               AND t.created_at = tp.created_at \
         JOIN accounts src ON src.id = t.source_id \
         LEFT JOIN LATERAL ( \
             SELECT array_agg(DISTINCT op_type_name(oa.type) \
                              ORDER BY op_type_name(oa.type)) AS operation_types \
             FROM operations_appearances oa \
             WHERE oa.transaction_id = t.id \
               AND oa.created_at     = t.created_at \
         ) ops ON TRUE \
         WHERE ($3::timestamptz IS NULL OR (tp.created_at, tp.transaction_id) < ($3, $4)) \
         ORDER BY tp.created_at DESC, tp.transaction_id DESC \
         LIMIT $2",
    )
    .bind(account_strkey)
    .bind(limit)
    .bind(cursor_ts)
    .bind(cursor_id)
    .fetch_all(pool)
    .await?;

    Ok(raw
        .iter()
        .map(|r| AccountTxRow {
            id: r.get("id"),
            hash: r.get("hash"),
            ledger_sequence: r.get("ledger_sequence"),
            application_order: r.get("application_order"),
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
