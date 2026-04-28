//! Database queries for the assets endpoints.
//!
//! Aligned with canonical SQL `endpoint-queries/{08,09,10}_*.sql` (task 0167).
//! Two deliberate divergences: (1) `:id` resolution stays at the API layer
//! (3 fetch_by_* paths, no surrogate-first single-SQL); (2) `/transactions`
//! is one OR'd query instead of canonical's split A/B variants. Both produce
//! the same result.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

use crate::common::cursor::TsIdCursor;

#[derive(Debug, Clone)]
pub struct AssetRow {
    pub id: i32,
    pub asset_type: i16,
    /// Pre-decoded via `token_asset_type_name()` SQL helper. `None` only
    /// when the discriminant is outside the schema CHECK range — defensive
    /// against future schema drift.
    pub asset_type_name: Option<String>,
    pub asset_code: Option<String>,
    /// Already resolved through `accounts.account_id` join.
    pub issuer: Option<String>,
    /// Already resolved through `soroban_contracts.contract_id` join.
    pub contract_id: Option<String>,
    pub name: Option<String>,
    pub total_supply: Option<String>,
    pub holder_count: Option<i32>,
    pub icon_url: Option<String>,
    /// `soroban_contracts.deployed_at_ledger` — populated for SAC and
    /// Soroban-native rows; `None` for native and classic_credit.
    pub deployed_at_ledger: Option<i64>,
}

#[derive(Debug)]
pub struct AssetTxRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
}

/// Pagination payload for `GET /v1/assets`. The `assets` table is
/// unpartitioned and has no `created_at`, so the project-default
/// `TsIdCursor` does not fit — natural order is `id DESC`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetIdCursor {
    pub id: i32,
}

pub struct ResolvedListParams {
    pub limit: i64,
    pub cursor: Option<AssetIdCursor>,
    pub asset_type: Option<i16>,
    /// Raw substring (no `%` / `_` from the caller). The SQL builder
    /// wraps it in `%...%` for the trigram match.
    pub asset_code: Option<String>,
}

fn push_glue(qb: &mut sqlx::QueryBuilder<'_, sqlx::Postgres>, has_where: &mut bool) {
    qb.push(if *has_where { " AND" } else { " WHERE" });
    *has_where = true;
}

const ASSET_SELECT: &str = "SELECT a.id, \
     token_asset_type_name(a.asset_type) AS asset_type_name, \
     a.asset_type AS asset_type, \
     a.asset_code, \
     iss.account_id AS issuer, \
     sc.contract_id, \
     a.name, \
     a.total_supply::text AS total_supply, \
     a.holder_count, \
     a.icon_url, \
     sc.deployed_at_ledger AS deployed_at_ledger \
     FROM assets a \
     LEFT JOIN accounts iss ON iss.id = a.issuer_id \
     LEFT JOIN soroban_contracts sc ON sc.id = a.contract_id";

fn map_asset_row(r: &PgRow) -> AssetRow {
    AssetRow {
        id: r.get("id"),
        asset_type: r.get("asset_type"),
        asset_type_name: r.get("asset_type_name"),
        asset_code: r.get("asset_code"),
        issuer: r.get("issuer"),
        contract_id: r.get("contract_id"),
        name: r.get("name"),
        total_supply: r.get("total_supply"),
        holder_count: r.get("holder_count"),
        icon_url: r.get("icon_url"),
        deployed_at_ledger: r.get("deployed_at_ledger"),
    }
}

pub async fn fetch_list(
    pool: &PgPool,
    params: &ResolvedListParams,
) -> Result<Vec<AssetRow>, sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(ASSET_SELECT);
    let mut has_where = false;

    if let Some(t) = params.asset_type {
        push_glue(&mut qb, &mut has_where);
        qb.push(" a.asset_type = ");
        qb.push_bind(t);
    }
    if let Some(code) = &params.asset_code {
        // Substring trigram match — leading `%` defeats btree but is served
        // by `idx_assets_code_trgm` (GIN gin_trgm_ops). The wrap happens here
        // so callers pass the raw substring (not a LIKE pattern).
        push_glue(&mut qb, &mut has_where);
        qb.push(" a.asset_code ILIKE '%' || ");
        qb.push_bind(code.as_str());
        qb.push(" || '%'");
    }
    if let Some(cursor) = &params.cursor {
        push_glue(&mut qb, &mut has_where);
        qb.push(" a.id < ");
        qb.push_bind(cursor.id);
    }

    qb.push(" ORDER BY a.id DESC LIMIT ");
    qb.push_bind(params.limit + 1);

    let raw: Vec<PgRow> = qb.build().fetch_all(pool).await?;
    Ok(raw.iter().map(map_asset_row).collect())
}

pub async fn fetch_by_id(pool: &PgPool, id: i32) -> Result<Option<AssetRow>, sqlx::Error> {
    let sql = format!("{ASSET_SELECT} WHERE a.id = $1");
    let raw: Option<PgRow> = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
    Ok(raw.as_ref().map(map_asset_row))
}

pub async fn fetch_by_contract_id(
    pool: &PgPool,
    contract_id: &str,
) -> Result<Option<AssetRow>, sqlx::Error> {
    let sql = format!("{ASSET_SELECT} WHERE sc.contract_id = $1");
    let raw: Option<PgRow> = sqlx::query(&sql)
        .bind(contract_id)
        .fetch_optional(pool)
        .await?;
    Ok(raw.as_ref().map(map_asset_row))
}

pub async fn fetch_by_code_issuer(
    pool: &PgPool,
    asset_code: &str,
    issuer_address: &str,
) -> Result<Option<AssetRow>, sqlx::Error> {
    let sql = format!("{ASSET_SELECT} WHERE a.asset_code = $1 AND iss.account_id = $2 LIMIT 1");
    let raw: Option<PgRow> = sqlx::query(&sql)
        .bind(asset_code)
        .bind(issuer_address)
        .fetch_optional(pool)
        .await?;
    Ok(raw.as_ref().map(map_asset_row))
}

/// Identity slice used by [`fetch_transactions`] to compose its predicate.
/// Native XLM (no identity at all) is filtered upstream by
/// [`asset_predicate_present`] so this struct never carries an empty triple.
pub struct AssetIdentity<'a> {
    pub asset_code: Option<&'a str>,
    pub issuer_address: Option<&'a str>,
    pub contract_id: Option<&'a str>,
}

/// Per-`asset_type` predicate composition (ADR 0037 §223–258 + 0038):
///   classic_credit         → `(asset_code, asset_issuer_id)`
///   sac (classic-wrap)     → classic identity OR `contract_id`
///   sac (native-wrap)      → `contract_id` only
///   soroban                → `contract_id` only
///   native                 → never reaches this fn (caller short-circuits)
pub async fn fetch_transactions(
    pool: &PgPool,
    identity: &AssetIdentity<'_>,
    limit: i64,
    cursor: Option<&TsIdCursor>,
) -> Result<Vec<AssetTxRow>, sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT DISTINCT t.id, encode(t.hash, 'hex') AS hash, t.ledger_sequence, \
         a.account_id AS source_account, t.successful, t.fee_charged, \
         t.created_at, t.operation_count, t.has_soroban, \
         COALESCE(ops.operation_types, ARRAY[]::text[]) AS operation_types \
         FROM transactions t \
         JOIN accounts a ON a.id = t.source_id \
         JOIN operations_appearances oa \
              ON oa.transaction_id = t.id AND oa.created_at = t.created_at \
         LEFT JOIN LATERAL ( \
             SELECT array_agg(DISTINCT op_type_name(oa2.type) ORDER BY op_type_name(oa2.type)) \
                    AS operation_types \
             FROM operations_appearances oa2 \
             WHERE oa2.transaction_id = t.id \
               AND oa2.created_at     = t.created_at \
         ) ops ON TRUE",
    );

    let has_classic = identity.asset_code.is_some() && identity.issuer_address.is_some();
    let has_contract = identity.contract_id.is_some();

    qb.push(" WHERE (");
    let mut wrote_branch = false;

    if has_classic {
        qb.push("(oa.asset_code = ");
        qb.push_bind(identity.asset_code.expect("guarded above"));
        qb.push(" AND oa.asset_issuer_id = (SELECT id FROM accounts WHERE account_id = ");
        qb.push_bind(identity.issuer_address.expect("guarded above"));
        qb.push("))");
        wrote_branch = true;
    }
    if has_contract {
        if wrote_branch {
            qb.push(" OR ");
        }
        qb.push("(oa.contract_id = (SELECT id FROM soroban_contracts WHERE contract_id = ");
        qb.push_bind(identity.contract_id.expect("guarded above"));
        qb.push("))");
    }
    qb.push(")");

    if let Some(c) = cursor {
        qb.push(" AND (t.created_at, t.id) < (");
        qb.push_bind(c.ts);
        qb.push(", ");
        qb.push_bind(c.id);
        qb.push(")");
    }

    qb.push(" ORDER BY t.created_at DESC, t.id DESC LIMIT ");
    qb.push_bind(limit + 1);

    let raw: Vec<PgRow> = qb.build().fetch_all(pool).await?;
    Ok(raw
        .iter()
        .map(|r| AssetTxRow {
            id: r.get("id"),
            hash: r.get("hash"),
            ledger_sequence: r.get("ledger_sequence"),
            source_account: r.get("source_account"),
            successful: r.get("successful"),
            fee_charged: r.get("fee_charged"),
            created_at: r.get("created_at"),
            operation_count: r.get("operation_count"),
            has_soroban: r.get("has_soroban"),
            operation_types: r.get("operation_types"),
        })
        .collect())
}

/// `false` for native XLM (no DB-side identity referenced by ops). Caller
/// short-circuits with an empty page so [`fetch_transactions`] never emits
/// a degenerate `WHERE ()` SQL.
pub fn asset_predicate_present(identity: &AssetIdentity<'_>) -> bool {
    let has_classic = identity.asset_code.is_some() && identity.issuer_address.is_some();
    let has_contract = identity.contract_id.is_some();
    has_classic || has_contract
}
