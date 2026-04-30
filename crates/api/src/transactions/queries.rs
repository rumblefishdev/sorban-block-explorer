//! Database queries for the transactions endpoints.

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use crate::common::cursor::TsIdCursor;

#[derive(Debug)]
pub struct TxListRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub source_account: String,
    pub fee_charged: i64,
    pub inner_tx_hash: Option<String>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
    pub contract_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct TxDetailRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub source_account: String,
    pub fee_charged: i64,
    pub inner_tx_hash: Option<String>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
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
    pub appearance_id: i64,
    pub type_name: String,
    pub op_type: i16,
    pub source_account: Option<String>,
    pub destination_account: Option<String>,
    pub contract_id: Option<String>,
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    pub pool_id: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct EventAppearanceRow {
    pub contract_id: String,
    pub ledger_sequence: i64,
    pub amount: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct InvocationAppearanceRow {
    pub contract_id: String,
    pub caller_account: Option<String>,
    pub ledger_sequence: i64,
    pub amount: i32,
    pub created_at: DateTime<Utc>,
}

pub struct ResolvedListParams {
    pub limit: i64,
    pub cursor: Option<TsIdCursor>,
    pub source_account: Option<String>,
    pub contract_id: Option<String>,
    pub op_type: Option<i16>,
}

fn push_glue(qb: &mut sqlx::QueryBuilder<'_, sqlx::Postgres>, has_where: &mut bool) {
    qb.push(if *has_where { " AND" } else { " WHERE" });
    *has_where = true;
}

fn push_cursor_predicate(qb: &mut sqlx::QueryBuilder<'_, sqlx::Postgres>, cursor: &TsIdCursor) {
    qb.push(" (t.created_at, t.id) < (");
    qb.push_bind(cursor.ts);
    qb.push(", ");
    qb.push_bind(cursor.id);
    qb.push(")");
}

fn map_list_row(r: &PgRow) -> TxListRow {
    TxListRow {
        id: r.get("id"),
        hash: r.get("hash"),
        ledger_sequence: r.get("ledger_sequence"),
        application_order: r.get("application_order"),
        source_account: r.get("source_account"),
        fee_charged: r.get("fee_charged"),
        inner_tx_hash: r.get("inner_tx_hash"),
        successful: r.get("successful"),
        operation_count: r.get("operation_count"),
        has_soroban: r.get("has_soroban"),
        operation_types: r.get("operation_types"),
        contract_ids: r.get("contract_ids"),
        created_at: r.get("created_at"),
    }
}

const LIST_PROJECTION: &str = "\
    SELECT \
        t.id, \
        encode(t.hash, 'hex')          AS hash, \
        t.ledger_sequence, \
        t.application_order, \
        src.account_id                 AS source_account, \
        t.fee_charged, \
        encode(t.inner_tx_hash, 'hex') AS inner_tx_hash, \
        t.successful, \
        t.operation_count, \
        t.has_soroban, \
        COALESCE(ops.operation_types, ARRAY[]::text[]) AS operation_types, \
        COALESCE(ctr.contract_ids,    ARRAY[]::text[]) AS contract_ids, \
        t.created_at\
    ";

const LIST_LATERAL_BLOCKS: &str = "\
    LEFT JOIN LATERAL ( \
        SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types \
        FROM operations_appearances oa \
        WHERE oa.transaction_id = t.id \
          AND oa.created_at     = t.created_at \
    ) ops ON TRUE \
    LEFT JOIN LATERAL ( \
        SELECT array_agg(DISTINCT sc.contract_id ORDER BY sc.contract_id) AS contract_ids \
        FROM ( \
            SELECT contract_id FROM operations_appearances \
            WHERE transaction_id = t.id \
              AND created_at     = t.created_at \
              AND contract_id IS NOT NULL \
            UNION \
            SELECT contract_id FROM soroban_invocations_appearances \
            WHERE transaction_id = t.id \
              AND created_at     = t.created_at \
            UNION \
            SELECT contract_id FROM soroban_events_appearances \
            WHERE transaction_id = t.id \
              AND created_at     = t.created_at \
        ) all_ctr \
        JOIN soroban_contracts sc ON sc.id = all_ctr.contract_id \
    ) ctr ON TRUE\
    ";

pub async fn fetch_list(
    pool: &PgPool,
    params: &ResolvedListParams,
) -> Result<Vec<TxListRow>, sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new("");

    match (&params.contract_id, params.op_type) {
        (Some(cid), op_type_opt) => {
            qb.push("WITH matched_tx AS (SELECT DISTINCT created_at, transaction_id FROM (");
            push_contract_union_arm(&mut qb, "operations_appearances", cid, &params.cursor);
            qb.push(" UNION ");
            push_contract_union_arm(
                &mut qb,
                "soroban_invocations_appearances",
                cid,
                &params.cursor,
            );
            qb.push(" UNION ");
            push_contract_union_arm(&mut qb, "soroban_events_appearances", cid, &params.cursor);

            qb.push(") u ORDER BY created_at DESC, transaction_id DESC LIMIT ");
            // 4× over-fetch — a single tx may appear in up to all three arms.
            qb.push_bind(params.limit * 4);
            qb.push(") ");

            qb.push(LIST_PROJECTION);
            qb.push(
                " FROM matched_tx m \
                 JOIN transactions t ON t.id = m.transaction_id AND t.created_at = m.created_at \
                 JOIN accounts src ON src.id = t.source_id ",
            );
            qb.push(LIST_LATERAL_BLOCKS);

            let mut has_where = false;
            if let Some(acct) = &params.source_account {
                push_glue(&mut qb, &mut has_where);
                push_source_predicate(&mut qb, acct);
            }
            if let Some(op_type) = op_type_opt {
                push_glue(&mut qb, &mut has_where);
                qb.push(
                    " EXISTS (SELECT 1 FROM operations_appearances oa2 \
                       WHERE oa2.transaction_id = t.id \
                         AND oa2.created_at     = t.created_at \
                         AND oa2.type           = ",
                );
                qb.push_bind(op_type);
                qb.push(")");
            }
        }

        (None, Some(op_type)) => {
            qb.push(
                "WITH matched_ops AS (\
                   SELECT DISTINCT ON (oa.created_at, oa.transaction_id) \
                          oa.transaction_id, oa.created_at \
                   FROM operations_appearances oa \
                   WHERE oa.type = ",
            );
            qb.push_bind(op_type);
            if let Some(c) = &params.cursor {
                qb.push(" AND (oa.created_at, oa.transaction_id) < (");
                qb.push_bind(c.ts);
                qb.push(", ");
                qb.push_bind(c.id);
                qb.push(")");
            }
            qb.push(" ORDER BY oa.created_at DESC, oa.transaction_id DESC, oa.id LIMIT ");
            qb.push_bind(params.limit * 4);
            qb.push(") ");

            qb.push(LIST_PROJECTION);
            qb.push(
                " FROM matched_ops m \
                 JOIN transactions t ON t.id = m.transaction_id AND t.created_at = m.created_at \
                 JOIN accounts src ON src.id = t.source_id ",
            );
            qb.push(LIST_LATERAL_BLOCKS);

            let mut has_where = false;
            if let Some(acct) = &params.source_account {
                push_glue(&mut qb, &mut has_where);
                push_source_predicate(&mut qb, acct);
            }
            // Re-applied: CTE LIMIT $1*4 may overshoot the cursor boundary.
            if let Some(cursor) = &params.cursor {
                push_glue(&mut qb, &mut has_where);
                push_cursor_predicate(&mut qb, cursor);
            }
        }

        (None, None) => {
            qb.push(LIST_PROJECTION);
            qb.push(" FROM transactions t JOIN accounts src ON src.id = t.source_id ");
            qb.push(LIST_LATERAL_BLOCKS);

            let mut has_where = false;
            if let Some(acct) = &params.source_account {
                push_glue(&mut qb, &mut has_where);
                push_source_predicate(&mut qb, acct);
            }
            if let Some(cursor) = &params.cursor {
                push_glue(&mut qb, &mut has_where);
                push_cursor_predicate(&mut qb, cursor);
            }
        }
    }

    qb.push(" ORDER BY t.created_at DESC, t.id DESC LIMIT ");
    qb.push_bind(params.limit + 1);

    let raw: Vec<PgRow> = qb.build().fetch_all(pool).await?;
    Ok(raw.iter().map(map_list_row).collect())
}

fn push_contract_union_arm<'q>(
    qb: &mut sqlx::QueryBuilder<'q, sqlx::Postgres>,
    table: &'static str,
    cid_strkey: &'q str,
    cursor: &Option<TsIdCursor>,
) {
    qb.push("SELECT created_at, transaction_id FROM ");
    qb.push(table);
    qb.push(" WHERE contract_id = (SELECT id FROM soroban_contracts WHERE contract_id = ");
    qb.push_bind(cid_strkey);
    qb.push(")");
    if let Some(c) = cursor {
        qb.push(" AND (created_at, transaction_id) < (");
        qb.push_bind(c.ts);
        qb.push(", ");
        qb.push_bind(c.id);
        qb.push(")");
    }
}

fn push_source_predicate<'q>(
    qb: &mut sqlx::QueryBuilder<'q, sqlx::Postgres>,
    acct_strkey: &'q str,
) {
    qb.push(" t.source_id = (SELECT id FROM accounts WHERE account_id = ");
    qb.push_bind(acct_strkey);
    qb.push(")");
}

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

pub async fn fetch_detail(
    pool: &PgPool,
    hash_bytes: &[u8],
    created_at: DateTime<Utc>,
) -> Result<Option<TxDetailRow>, sqlx::Error> {
    let raw: Option<PgRow> = sqlx::query(
        "SELECT \
            t.id, \
            encode(t.hash, 'hex')          AS hash, \
            t.ledger_sequence, \
            t.application_order, \
            a.account_id                   AS source_account, \
            t.fee_charged, \
            encode(t.inner_tx_hash, 'hex') AS inner_tx_hash, \
            t.successful, \
            t.operation_count, \
            t.has_soroban, \
            t.created_at, \
            t.parse_error \
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
        application_order: r.get("application_order"),
        source_account: r.get("source_account"),
        fee_charged: r.get("fee_charged"),
        inner_tx_hash: r.get("inner_tx_hash"),
        successful: r.get("successful"),
        operation_count: r.get("operation_count"),
        has_soroban: r.get("has_soroban"),
        created_at: r.get("created_at"),
        parse_error: r.get("parse_error"),
    }))
}

pub async fn fetch_operations(
    pool: &PgPool,
    transaction_id: i64,
    created_at: DateTime<Utc>,
) -> Result<Vec<OpRow>, sqlx::Error> {
    // ORDER BY oa.id: BIGSERIAL is monotone with ingest = within-tx
    // application order, so result-set position is the operation's index.
    let raw: Vec<PgRow> = sqlx::query(
        "SELECT \
            oa.id                           AS appearance_id, \
            op_type_name(oa.type)           AS type_name, \
            oa.type                         AS op_type, \
            src.account_id                  AS source_account, \
            dst.account_id                  AS destination_account, \
            sc.contract_id, \
            oa.asset_code, \
            iss.account_id                  AS asset_issuer, \
            encode(oa.pool_id, 'hex')       AS pool_id, \
            oa.ledger_sequence, \
            oa.created_at \
         FROM operations_appearances oa \
         LEFT JOIN accounts          src ON src.id = oa.source_id \
         LEFT JOIN accounts          dst ON dst.id = oa.destination_id \
         LEFT JOIN soroban_contracts sc  ON sc.id  = oa.contract_id \
         LEFT JOIN accounts          iss ON iss.id = oa.asset_issuer_id \
         WHERE oa.transaction_id = $1 AND oa.created_at = $2 \
         ORDER BY oa.id",
    )
    .bind(transaction_id)
    .bind(created_at)
    .fetch_all(pool)
    .await?;

    Ok(raw
        .iter()
        .map(|r| OpRow {
            appearance_id: r.get("appearance_id"),
            type_name: r.get("type_name"),
            op_type: r.get("op_type"),
            source_account: r.get("source_account"),
            destination_account: r.get("destination_account"),
            contract_id: r.get("contract_id"),
            asset_code: r.get("asset_code"),
            asset_issuer: r.get("asset_issuer"),
            pool_id: r.get("pool_id"),
            ledger_sequence: r.get("ledger_sequence"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub async fn fetch_participants(
    pool: &PgPool,
    transaction_id: i64,
    created_at: DateTime<Utc>,
) -> Result<Vec<String>, sqlx::Error> {
    let raw: Vec<PgRow> = sqlx::query(
        "SELECT a.account_id \
         FROM transaction_participants tp \
         JOIN accounts a ON a.id = tp.account_id \
         WHERE tp.transaction_id = $1 AND tp.created_at = $2 \
         ORDER BY a.account_id",
    )
    .bind(transaction_id)
    .bind(created_at)
    .fetch_all(pool)
    .await?;

    Ok(raw.iter().map(|r| r.get("account_id")).collect())
}

pub async fn fetch_event_appearances(
    pool: &PgPool,
    transaction_id: i64,
    created_at: DateTime<Utc>,
) -> Result<Vec<EventAppearanceRow>, sqlx::Error> {
    let raw: Vec<PgRow> = sqlx::query(
        "SELECT \
            sc.contract_id, \
            sea.ledger_sequence, \
            sea.amount, \
            sea.created_at \
         FROM soroban_events_appearances sea \
         JOIN soroban_contracts sc ON sc.id = sea.contract_id \
         WHERE sea.transaction_id = $1 AND sea.created_at = $2 \
         ORDER BY sea.ledger_sequence, sc.contract_id",
    )
    .bind(transaction_id)
    .bind(created_at)
    .fetch_all(pool)
    .await?;

    Ok(raw
        .iter()
        .map(|r| EventAppearanceRow {
            contract_id: r.get("contract_id"),
            ledger_sequence: r.get("ledger_sequence"),
            amount: r.get("amount"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub async fn fetch_invocation_appearances(
    pool: &PgPool,
    transaction_id: i64,
    created_at: DateTime<Utc>,
) -> Result<Vec<InvocationAppearanceRow>, sqlx::Error> {
    let raw: Vec<PgRow> = sqlx::query(
        "SELECT \
            sc.contract_id, \
            caller.account_id    AS caller_account, \
            sia.ledger_sequence, \
            sia.amount, \
            sia.created_at \
         FROM soroban_invocations_appearances sia \
         JOIN soroban_contracts sc      ON sc.id     = sia.contract_id \
         LEFT JOIN accounts      caller ON caller.id = sia.caller_id \
         WHERE sia.transaction_id = $1 AND sia.created_at = $2 \
         ORDER BY sia.ledger_sequence, sc.contract_id",
    )
    .bind(transaction_id)
    .bind(created_at)
    .fetch_all(pool)
    .await?;

    Ok(raw
        .iter()
        .map(|r| InvocationAppearanceRow {
            contract_id: r.get("contract_id"),
            caller_account: r.get("caller_account"),
            ledger_sequence: r.get("ledger_sequence"),
            amount: r.get("amount"),
            created_at: r.get("created_at"),
        })
        .collect())
}
