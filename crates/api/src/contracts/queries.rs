//! Database queries for the contracts endpoints.
//!
//! Schema reference: ADR 0037 (live snapshot). All Soroban detail tables are
//! appearance indexes per ADRs 0033 / 0034 — the queries below pull just the
//! identity + index columns; per-event / per-invocation detail is reconstructed
//! from XDR by the handlers via the public Stellar archive (ADR 0029).

use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

// ---------------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ContractRow {
    /// Surrogate `BIGINT` PK (ADR 0030). Used for FK joins on appearance tables.
    pub id: i64,
    pub contract_id: String,
    /// WASM hash hex (64 chars), `None` for SAC / pre-upload contracts.
    pub wasm_hash: Option<String>,
    pub deployer_account: Option<String>,
    pub deployed_at_ledger: Option<i64>,
    pub contract_type: Option<i16>,
    pub is_sac: bool,
    pub metadata: Option<serde_json::Value>,
}

pub async fn fetch_contract(
    pool: &PgPool,
    contract_id: &str,
) -> Result<Option<ContractRow>, sqlx::Error> {
    let row: Option<PgRow> = sqlx::query(
        "SELECT sc.id, sc.contract_id, encode(sc.wasm_hash, 'hex') AS wasm_hash, \
         a.account_id AS deployer_account, sc.deployed_at_ledger, sc.contract_type, \
         sc.is_sac, sc.metadata \
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
        deployer_account: r.get("deployer_account"),
        deployed_at_ledger: r.get("deployed_at_ledger"),
        contract_type: r.get("contract_type"),
        is_sac: r.get("is_sac"),
        metadata: r.get("metadata"),
    }))
}

/// Aggregate `(invocation_count, event_count)` over the appearance indexes.
/// Both casts (`SUM(amount)::BIGINT`) clamp the NUMERIC return type that
/// Postgres uses for `SUM(BIGINT)` back to `i64` — well within range for
/// any single contract on pubnet (event/invocation totals fit in i63).
pub async fn fetch_contract_stats(
    pool: &PgPool,
    contract_surrogate_id: i64,
) -> Result<(i64, i64), sqlx::Error> {
    let invocation_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount)::BIGINT, 0) \
         FROM soroban_invocations_appearances WHERE contract_id = $1",
    )
    .bind(contract_surrogate_id)
    .fetch_one(pool)
    .await?;

    let event_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount)::BIGINT, 0) \
         FROM soroban_events_appearances WHERE contract_id = $1",
    )
    .bind(contract_surrogate_id)
    .fetch_one(pool)
    .await?;

    Ok((invocation_count, event_count))
}

// ---------------------------------------------------------------------------
// Interface
// ---------------------------------------------------------------------------

/// Fetch the WASM-spec metadata blob for a contract.
///
/// Returns `Ok(None)` when:
/// - the contract row is missing,
/// - the contract has no `wasm_hash` (SAC / pre-upload),
/// - no matching `wasm_interface_metadata` row exists yet, or
/// - the matching row is a **stub** (`metadata = '{}'::jsonb`, no
///   `functions` key) — these are inserted by `stub_unknown_wasm_interfaces`
///   in the indexer to satisfy the `soroban_contracts.wasm_hash` FK during
///   mid-stream backfill (task 0153) and carry no real interface payload.
///
/// The handler maps all four cases to a 404 — there is no public interface
/// to surface in any of them. The `metadata ? 'functions'` JSONB key-exists
/// predicate is what distinguishes stubs from real interfaces (real ones
/// always carry a `functions` array, even if empty).
pub async fn fetch_wasm_interface(
    pool: &PgPool,
    contract_id: &str,
) -> Result<Option<serde_json::Value>, sqlx::Error> {
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT wim.metadata \
         FROM soroban_contracts sc \
         JOIN wasm_interface_metadata wim ON wim.wasm_hash = sc.wasm_hash \
         WHERE sc.contract_id = $1 AND wim.metadata ? 'functions'",
    )
    .bind(contract_id)
    .fetch_optional(pool)
    .await
}

// ---------------------------------------------------------------------------
// Invocations appearance page
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct InvocationAppearanceRow {
    pub transaction_id: i64,
    /// 64-char lowercase hex.
    pub transaction_hash: String,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

/// Fetch a page of `(contract, transaction, ledger)` invocation appearances.
///
/// Per-node detail (function name, args, return value, caller_account) is
/// re-extracted from XDR by the handler — `soroban_invocations_appearances.caller_id`
/// only carries the root-level caller and is intentionally not surfaced here.
///
/// Pagination key is `(created_at DESC, transaction_id DESC)` — same as the
/// transactions module — to enable partition pruning on `created_at` (the
/// table's range partition key) and to share cursor encoding semantics
/// across modules. The current `idx_sia_contract_ledger
/// (contract_id, ledger_sequence DESC)` does not perfectly match this
/// ordering, so the planner currently sorts after the index seek; the
/// matching index is tracked under task 0132 (DB: add missing indexes for
/// planned API query patterns) so this module does not own the migration.
pub async fn fetch_invocation_appearances(
    pool: &PgPool,
    contract_surrogate_id: i64,
    limit: i64,
    cursor: Option<(DateTime<Utc>, i64)>,
) -> Result<Vec<InvocationAppearanceRow>, sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT sia.transaction_id, encode(t.hash, 'hex') AS tx_hash, \
         sia.ledger_sequence, sia.created_at \
         FROM soroban_invocations_appearances sia \
         JOIN transactions t ON t.id = sia.transaction_id AND t.created_at = sia.created_at \
         WHERE sia.contract_id = ",
    );
    qb.push_bind(contract_surrogate_id);
    if let Some((cursor_ts, cursor_id)) = cursor {
        qb.push(" AND (sia.created_at, sia.transaction_id) < (");
        qb.push_bind(cursor_ts);
        qb.push(", ");
        qb.push_bind(cursor_id);
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
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Events appearance page
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct EventAppearanceRow {
    pub transaction_id: i64,
    /// 64-char lowercase hex.
    pub transaction_hash: String,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

/// Fetch a page of contract event appearances.
///
/// Pagination key is `(created_at DESC, transaction_id DESC)` for the same
/// reasons as `fetch_invocation_appearances` (partition pruning + shared
/// cursor semantics). `idx_sea_contract_ledger
/// (contract_id, ledger_sequence DESC, created_at DESC)` already includes
/// `created_at` so the planner can use it for ordering after the
/// `contract_id` seek; the missing trailing `transaction_id` is a small
/// secondary sort and is tracked alongside the invocations index under
/// task 0132.
pub async fn fetch_event_appearances(
    pool: &PgPool,
    contract_surrogate_id: i64,
    limit: i64,
    cursor: Option<(DateTime<Utc>, i64)>,
) -> Result<Vec<EventAppearanceRow>, sqlx::Error> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT sea.transaction_id, encode(t.hash, 'hex') AS tx_hash, \
         sea.ledger_sequence, sea.created_at \
         FROM soroban_events_appearances sea \
         JOIN transactions t ON t.id = sea.transaction_id AND t.created_at = sea.created_at \
         WHERE sea.contract_id = ",
    );
    qb.push_bind(contract_surrogate_id);
    if let Some((cursor_ts, cursor_id)) = cursor {
        qb.push(" AND (sea.created_at, sea.transaction_id) < (");
        qb.push_bind(cursor_ts);
        qb.push(", ");
        qb.push_bind(cursor_id);
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
        })
        .collect())
}
