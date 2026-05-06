//! Database queries for the contracts endpoints.
//! Mirrors canonical SQL `endpoint-queries/{11..14}_*.sql` (task 0167).

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use crate::common::cursor::TsIdCursor;

#[derive(Debug)]
pub struct ContractRow {
    pub id: i64,
    pub contract_id: String,
    pub wasm_hash: Option<String>,
    pub wasm_uploaded_at_ledger: Option<i64>,
    pub deployer: Option<String>,
    pub deployed_at_ledger: Option<i64>,
    pub contract_type_name: Option<String>,
    pub contract_type: Option<i16>,
    pub is_sac: bool,
}

pub async fn fetch_contract(
    pool: &PgPool,
    contract_id: &str,
) -> Result<Option<ContractRow>, sqlx::Error> {
    // Per ADR 0042 / task 0156: `soroban_contracts.metadata JSONB` was
    // replaced by typed `name VARCHAR(256)`. The detail response no
    // longer projects a `metadata` field (was always `{}` in practice
    // and added no value). The `name` column is consumed only by the
    // search query (`COALESCE(sc.name, '')`); detail page does not
    // surface it as a separate field.
    let row: Option<PgRow> = sqlx::query(
        "SELECT sc.id, sc.contract_id, encode(sc.wasm_hash, 'hex') AS wasm_hash, \
         sc.wasm_uploaded_at_ledger, \
         a.account_id AS deployer, sc.deployed_at_ledger, \
         contract_type_name(sc.contract_type) AS contract_type_name, \
         sc.contract_type, sc.is_sac \
         FROM soroban_contracts sc \
         LEFT JOIN accounts a ON a.id = sc.deployer_id \
         WHERE sc.contract_id = $1",
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ContractRow {
        id: r.get("id"),
        contract_id: r.get("contract_id"),
        wasm_hash: r.get("wasm_hash"),
        wasm_uploaded_at_ledger: r.get("wasm_uploaded_at_ledger"),
        deployer: r.get("deployer"),
        deployed_at_ledger: r.get("deployed_at_ledger"),
        contract_type_name: r.get("contract_type_name"),
        contract_type: r.get("contract_type"),
        is_sac: r.get("is_sac"),
    }))
}

/// Bounded-window stats per canonical 11 Statement B. `window` is bound
/// twice — `$2::interval` for the predicate, `$2::text` for the echoed
/// label. Drops the unbounded `SUM(amount)` over events table that the
/// task 0167 audit flagged as a HIGH-severity full-history scan.
pub async fn fetch_contract_stats(
    pool: &PgPool,
    contract_surrogate_id: i64,
    window: &str,
) -> Result<(i64, i64, String), sqlx::Error> {
    let row: PgRow = sqlx::query(
        "SELECT COUNT(*)::BIGINT                          AS recent_invocations, \
                COUNT(DISTINCT caller_id)::BIGINT         AS recent_unique_callers, \
                $2::text                                  AS stats_window \
         FROM soroban_invocations_appearances \
         WHERE contract_id = $1 \
           AND created_at >= NOW() - $2::interval",
    )
    .bind(contract_surrogate_id)
    .bind(window)
    .fetch_one(pool)
    .await?;

    Ok((
        row.get("recent_invocations"),
        row.get("recent_unique_callers"),
        row.get("stats_window"),
    ))
}

#[derive(Debug)]
pub struct InterfaceRow {
    pub contract_id: String,
    pub wasm_hash: Option<String>,
    /// `None` for SAC / pre-upload / stub rows. The CASE predicate filters
    /// stubs (`metadata = '{}'::jsonb`, no `functions` key — task 0153)
    /// while preserving canonical 12's "always project the contract row"
    /// LEFT JOIN model.
    pub interface_metadata: Option<serde_json::Value>,
}

/// `Ok(None)` only when the contract row itself is missing; SAC and
/// pre-upload contracts return `Ok(Some(_))` with `interface_metadata = None`.
pub async fn fetch_wasm_interface(
    pool: &PgPool,
    contract_id: &str,
) -> Result<Option<InterfaceRow>, sqlx::Error> {
    let row: Option<PgRow> = sqlx::query(
        "SELECT sc.contract_id, \
                encode(sc.wasm_hash, 'hex') AS wasm_hash, \
                CASE WHEN wim.metadata ? 'functions' \
                     THEN wim.metadata \
                     ELSE NULL END           AS interface_metadata \
         FROM soroban_contracts sc \
         LEFT JOIN wasm_interface_metadata wim ON wim.wasm_hash = sc.wasm_hash \
         WHERE sc.contract_id = $1",
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| InterfaceRow {
        contract_id: r.get("contract_id"),
        wasm_hash: r.get("wasm_hash"),
        interface_metadata: r.get("interface_metadata"),
    }))
}

#[derive(Debug)]
pub struct InvocationAppearanceRow {
    pub transaction_id: i64,
    pub transaction_hash: String,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
    pub caller_account: Option<String>,
    pub amount: i32,
    pub successful: bool,
}

/// Pure DB per canonical 13 — function name / args / return value live
/// on E3. Sort key `(created_at DESC, transaction_id DESC)`; matching
/// index tracked under task 0132.
pub async fn fetch_invocation_appearances(
    pool: &PgPool,
    contract_surrogate_id: i64,
    limit: i64,
    cursor: Option<&TsIdCursor>,
) -> Result<Vec<InvocationAppearanceRow>, sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT sia.transaction_id, \
                encode(t.hash, 'hex')   AS tx_hash, \
                sia.ledger_sequence, \
                sia.created_at, \
                caller.account_id        AS caller_account, \
                sia.amount, \
                t.successful \
         FROM soroban_invocations_appearances sia \
         JOIN transactions t ON t.id = sia.transaction_id AND t.created_at = sia.created_at \
         LEFT JOIN accounts caller ON caller.id = sia.caller_id \
         WHERE sia.contract_id = ",
    );
    qb.push_bind(contract_surrogate_id);
    if let Some(c) = cursor {
        qb.push(" AND (sia.created_at, sia.transaction_id) < (");
        qb.push_bind(c.ts);
        qb.push(", ");
        qb.push_bind(c.id);
        qb.push(")");
    }
    qb.push(" ORDER BY sia.created_at DESC, sia.transaction_id DESC LIMIT ");
    qb.push_bind(limit + 1);

    let raw: Vec<PgRow> = qb.build().fetch_all(pool).await?;
    Ok(raw
        .iter()
        .map(|r| InvocationAppearanceRow {
            transaction_id: r.get("transaction_id"),
            transaction_hash: r.get("tx_hash"),
            ledger_sequence: r.get("ledger_sequence"),
            created_at: r.get("created_at"),
            caller_account: r.get("caller_account"),
            amount: r.get("amount"),
            successful: r.get("successful"),
        })
        .collect())
}

#[derive(Debug)]
pub struct EventAppearanceRow {
    pub transaction_id: i64,
    pub transaction_hash: String,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
    pub successful: bool,
    pub amount: i64,
}

/// DB appearance index per canonical 14. The handler overlays archive
/// XDR (ADR 0033) to surface one wire row per event.
pub async fn fetch_event_appearances(
    pool: &PgPool,
    contract_surrogate_id: i64,
    limit: i64,
    cursor: Option<&TsIdCursor>,
) -> Result<Vec<EventAppearanceRow>, sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT sea.transaction_id, \
                encode(t.hash, 'hex')   AS tx_hash, \
                sea.ledger_sequence, \
                sea.created_at, \
                t.successful, \
                sea.amount \
         FROM soroban_events_appearances sea \
         JOIN transactions t ON t.id = sea.transaction_id AND t.created_at = sea.created_at \
         WHERE sea.contract_id = ",
    );
    qb.push_bind(contract_surrogate_id);
    if let Some(c) = cursor {
        qb.push(" AND (sea.created_at, sea.transaction_id) < (");
        qb.push_bind(c.ts);
        qb.push(", ");
        qb.push_bind(c.id);
        qb.push(")");
    }
    qb.push(" ORDER BY sea.created_at DESC, sea.transaction_id DESC LIMIT ");
    qb.push_bind(limit + 1);

    let raw: Vec<PgRow> = qb.build().fetch_all(pool).await?;
    Ok(raw
        .iter()
        .map(|r| EventAppearanceRow {
            transaction_id: r.get("transaction_id"),
            transaction_hash: r.get("tx_hash"),
            ledger_sequence: r.get("ledger_sequence"),
            created_at: r.get("created_at"),
            successful: r.get("successful"),
            amount: r.get("amount"),
        })
        .collect())
}
