//! Database queries for the transactions endpoints.

use chrono::{DateTime, Utc};
use domain::OperationType;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

// ---------------------------------------------------------------------------
// Internal row structs (not exposed in API response types)
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct TxListRow {
    pub id: i64,
    /// Lowercase hex (64 chars).
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    pub operation_count: i16,
}

#[derive(Debug)]
pub struct TxDetailRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    pub parse_error: bool,
}

#[derive(Debug)]
pub struct HashIndexRow {
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct OpRow {
    pub application_order: i16,
    pub op_type: i16,
    pub contract_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Resolved (pre-validated) list parameters used to build the dynamic query
// ---------------------------------------------------------------------------

pub struct ResolvedListParams {
    pub limit: i64,
    pub cursor: Option<(DateTime<Utc>, i64)>,
    /// `filter[source_account]` StrKey string (validated non-empty).
    pub source_account: Option<String>,
    /// `filter[contract_id]` StrKey string (validated non-empty).
    pub contract_id: Option<String>,
    /// `filter[operation_type]` mapped to SMALLINT via `domain::OperationType`.
    pub op_type: Option<i16>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn map_list_row(r: &PgRow) -> TxListRow {
    TxListRow {
        id: r.get("id"),
        hash: r.get("hash"),
        ledger_sequence: r.get("ledger_sequence"),
        source_account: r.get("source_account"),
        successful: r.get("successful"),
        fee_charged: r.get("fee_charged"),
        created_at: r.get("created_at"),
        operation_count: r.get("operation_count"),
    }
}

/// Parse `filter[operation_type]` string into the corresponding `i16` discriminant.
/// Returns `Err` for unknown type names.
pub fn parse_op_type(s: &str) -> Result<i16, ()> {
    s.parse::<OperationType>().map(|t| t as i16).map_err(|_| ())
}

// ---------------------------------------------------------------------------
// List query (dynamic WHERE via QueryBuilder)
// ---------------------------------------------------------------------------

pub async fn fetch_list(
    pool: &PgPool,
    params: &ResolvedListParams,
) -> Result<Vec<TxListRow>, sqlx::Error> {
    // Whether we need to join operations (contract_id or op_type filter).
    let needs_ops_join = params.contract_id.is_some() || params.op_type.is_some();

    let select = if needs_ops_join {
        // DISTINCT eliminates duplicate tx rows when multiple operations match.
        "SELECT DISTINCT t.id, encode(t.hash, 'hex') AS hash, t.ledger_sequence, \
         a.account_id AS source_account, t.successful, t.fee_charged, \
         t.created_at, t.operation_count \
         FROM transactions t \
         JOIN accounts a ON a.id = t.source_id \
         JOIN operations o ON o.transaction_id = t.id AND o.created_at = t.created_at"
    } else {
        "SELECT t.id, encode(t.hash, 'hex') AS hash, t.ledger_sequence, \
         a.account_id AS source_account, t.successful, t.fee_charged, \
         t.created_at, t.operation_count \
         FROM transactions t \
         JOIN accounts a ON a.id = t.source_id"
    };

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(select);

    // contract_id filter: join soroban_contracts and filter by StrKey.
    if let Some(cid) = &params.contract_id {
        qb.push(" LEFT JOIN soroban_contracts sc ON sc.id = o.contract_id");
        qb.push(" WHERE sc.contract_id = ");
        qb.push_bind(cid.as_str());
    }

    let mut has_where = params.contract_id.is_some();

    // source_account filter via subquery.
    if let Some(acct) = &params.source_account {
        qb.push(if has_where { " AND" } else { " WHERE" });
        qb.push(" t.source_id = (SELECT id FROM accounts WHERE account_id = ");
        qb.push_bind(acct.as_str());
        qb.push(")");
        has_where = true;
    }

    // op_type filter.
    if let Some(op_type) = params.op_type {
        qb.push(if has_where { " AND" } else { " WHERE" });
        qb.push(" o.type = ");
        qb.push_bind(op_type);
        has_where = true;
    }

    // Cursor predicate.
    if let Some((cursor_ts, cursor_id)) = params.cursor {
        qb.push(if has_where { " AND" } else { " WHERE" });
        qb.push(" (t.created_at, t.id) < (");
        qb.push_bind(cursor_ts);
        qb.push(", ");
        qb.push_bind(cursor_id);
        qb.push(")");
    }

    qb.push(" ORDER BY t.created_at DESC, t.id DESC LIMIT ");
    qb.push_bind(params.limit + 1); // fetch one extra to determine has_more

    let raw: Vec<PgRow> = qb.build().fetch_all(pool).await?;
    Ok(raw.iter().map(map_list_row).collect())
}

// ---------------------------------------------------------------------------
// Hash index lookup (for detail endpoint)
// ---------------------------------------------------------------------------

/// Look up a transaction's `(ledger_sequence, created_at)` from the unpartitioned
/// hash index table. Returns `None` when the hash is not found.
pub async fn lookup_hash_index(
    pool: &PgPool,
    hash_bytes: &[u8],
) -> Result<Option<HashIndexRow>, sqlx::Error> {
    let raw: Option<PgRow> = sqlx::query(
        "SELECT ledger_sequence, created_at FROM transaction_hash_index WHERE hash = $1",
    )
    .bind(hash_bytes)
    .fetch_optional(pool)
    .await?;

    Ok(raw.map(|r| HashIndexRow {
        ledger_sequence: r.get("ledger_sequence"),
        created_at: r.get("created_at"),
    }))
}

// ---------------------------------------------------------------------------
// Detail fetch
// ---------------------------------------------------------------------------

/// Fetch the full transaction row. Requires `created_at` from the hash index
/// so the planner can prune partitions.
pub async fn fetch_detail(
    pool: &PgPool,
    hash_bytes: &[u8],
    created_at: DateTime<Utc>,
) -> Result<Option<TxDetailRow>, sqlx::Error> {
    let raw: Option<PgRow> = sqlx::query(
        "SELECT t.id, encode(t.hash, 'hex') AS hash, t.ledger_sequence, \
         a.account_id AS source_account, t.successful, t.fee_charged, \
         t.created_at, t.parse_error \
         FROM transactions t \
         JOIN accounts a ON a.id = t.source_id \
         WHERE t.hash = $1 AND t.created_at = $2",
    )
    .bind(hash_bytes)
    .bind(created_at)
    .fetch_optional(pool)
    .await?;

    Ok(raw.map(|r| TxDetailRow {
        id: r.get("id"),
        hash: r.get("hash"),
        ledger_sequence: r.get("ledger_sequence"),
        source_account: r.get("source_account"),
        successful: r.get("successful"),
        fee_charged: r.get("fee_charged"),
        created_at: r.get("created_at"),
        parse_error: r.get("parse_error"),
    }))
}

// ---------------------------------------------------------------------------
// Operations for detail endpoint
// ---------------------------------------------------------------------------

pub async fn fetch_operations(
    pool: &PgPool,
    transaction_id: i64,
    created_at: DateTime<Utc>,
) -> Result<Vec<OpRow>, sqlx::Error> {
    let raw: Vec<PgRow> = sqlx::query(
        "SELECT o.application_order, o.type AS op_type, sc.contract_id \
         FROM operations o \
         LEFT JOIN soroban_contracts sc ON sc.id = o.contract_id \
         WHERE o.transaction_id = $1 AND o.created_at = $2 \
         ORDER BY o.application_order",
    )
    .bind(transaction_id)
    .bind(created_at)
    .fetch_all(pool)
    .await?;

    Ok(raw
        .iter()
        .map(|r| OpRow {
            application_order: r.get("application_order"),
            op_type: r.get("op_type"),
            contract_id: r.get("contract_id"),
        })
        .collect())
}
