//! Database queries for the ledgers endpoints.
//!
//! Aligned with canonical SQL `endpoint-queries/{04,05}_*.sql` (task 0167).
//!
//! - **List (`fetch_list`)** uses a single static query — no dynamic
//!   filters apply to ledgers. Cursor predicate is inlined as a row-value
//!   comparison so the planner walks `idx_ledgers_closed_at` in DESC.
//! - **Detail header (`fetch_by_sequence`)** computes `prev_sequence` /
//!   `next_sequence` via two `LATERAL ... LIMIT 1` lookups on the
//!   `ledgers` PK using `sequence < l.sequence` / `sequence > l.sequence`
//!   (PK ordering). Each costs one index-only seek; cheaper than a
//!   window over the whole table, and avoids the heap fetch that the
//!   secondary `idx_ledgers_closed_at` would require for projecting
//!   `sequence`.
//! - **Embedded transactions (`fetch_transactions`)** pulls the seven
//!   DB-side fields of `TransactionListItem` for a single ledger.
//!   Partition pruning is total: `created_at = $closed_at` (carried
//!   forward from the header query) is full equality, so only the
//!   monthly partition that owns this ledger's transactions is touched.
//!   This endpoint is DB-only — list rows do not carry memo / heavy
//!   fields, those live on the transaction detail endpoint instead.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::common::cursor::TsIdCursor;
use crate::transactions::dto::TransactionListItem;

use super::dto::LedgerListItem;

// ---------------------------------------------------------------------------
// Internal row structs (not exposed in API response types)
// ---------------------------------------------------------------------------

/// Detail header projection. Same fields as `LedgerListItem` plus the
/// LATERAL-derived navigation pair. Kept separate from the public DTO
/// because the response type composes this with an embedded paginated
/// list (`transactions`) that does not come from a single SQL row.
#[derive(Debug, sqlx::FromRow)]
pub struct LedgerDetailRow {
    pub sequence: i64,
    pub hash: String,
    pub closed_at: DateTime<Utc>,
    pub protocol_version: i32,
    pub transaction_count: i32,
    pub base_fee: i64,
    pub prev_sequence: Option<i64>,
    pub next_sequence: Option<i64>,
}

/// DB-side projection of an embedded transaction row.
///
/// Mirrors `transactions::queries::TxListRow` so both entry points
/// (`GET /v1/transactions` and `GET /v1/ledgers/:seq/transactions`)
/// produce a `TransactionListItem` with identical canonical-aligned
/// fields. `id` is the internal cursor tie-break, not exposed on the DTO.
#[derive(Debug, sqlx::FromRow)]
pub struct LedgerTxRow {
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

impl From<LedgerTxRow> for TransactionListItem {
    fn from(row: LedgerTxRow) -> Self {
        Self {
            hash: row.hash,
            ledger_sequence: row.ledger_sequence,
            application_order: row.application_order,
            source_account: row.source_account,
            fee_charged: row.fee_charged,
            inner_tx_hash: row.inner_tx_hash,
            successful: row.successful,
            operation_count: row.operation_count,
            has_soroban: row.has_soroban,
            operation_types: row.operation_types,
            contract_ids: row.contract_ids,
            created_at: row.created_at,
        }
    }
}

// ---------------------------------------------------------------------------
// List query — `GET /v1/ledgers`
// ---------------------------------------------------------------------------

/// Fetch one page of ledgers ordered by `(closed_at DESC, sequence DESC)`.
///
/// `limit` is the requested page size; the caller is expected to pass
/// `limit + 1` so the pagination layer can detect `has_more`. Cursor is
/// the project-default `TsIdCursor` — `cursor.ts` carries the row's
/// `closed_at` and `cursor.id` carries the row's `sequence`. Mapping is
/// fine because cursors are opaque per ADR 0008 (clients never construct
/// the payload by hand).
pub async fn fetch_list(
    pool: &PgPool,
    limit: i64,
    cursor: Option<&TsIdCursor>,
) -> Result<Vec<LedgerListItem>, sqlx::Error> {
    let cursor_closed_at = cursor.map(|c| c.ts);
    let cursor_sequence = cursor.map(|c| c.id);

    sqlx::query_as::<_, LedgerListItem>(
        "SELECT \
            l.sequence, \
            encode(l.hash, 'hex')   AS hash, \
            l.closed_at, \
            l.protocol_version, \
            l.transaction_count, \
            l.base_fee \
        FROM ledgers l \
        WHERE $2::timestamptz IS NULL \
           OR (l.closed_at, l.sequence) < ($2, $3) \
        ORDER BY l.closed_at DESC, l.sequence DESC \
        LIMIT $1",
    )
    .bind(limit)
    .bind(cursor_closed_at)
    .bind(cursor_sequence)
    .fetch_all(pool)
    .await
}

// ---------------------------------------------------------------------------
// Detail header query — `GET /v1/ledgers/:sequence`
// ---------------------------------------------------------------------------

/// Fetch the ledger header row plus `prev_sequence` / `next_sequence` via
/// LATERAL lookups on the `ledgers` PK (`sequence` ordering — index-only
/// scan, no heap fetch). Returns `Ok(None)` when no ledger has the
/// requested sequence (handler maps to 404).
pub async fn fetch_by_sequence(
    pool: &PgPool,
    sequence: i64,
) -> Result<Option<LedgerDetailRow>, sqlx::Error> {
    sqlx::query_as::<_, LedgerDetailRow>(
        "SELECT \
            l.sequence, \
            encode(l.hash, 'hex')   AS hash, \
            l.closed_at, \
            l.protocol_version, \
            l.transaction_count, \
            l.base_fee, \
            prev.sequence           AS prev_sequence, \
            nxt.sequence            AS next_sequence \
        FROM ledgers l \
        LEFT JOIN LATERAL ( \
            SELECT sequence \
            FROM ledgers \
            WHERE sequence < l.sequence \
            ORDER BY sequence DESC \
            LIMIT 1 \
        ) prev ON TRUE \
        LEFT JOIN LATERAL ( \
            SELECT sequence \
            FROM ledgers \
            WHERE sequence > l.sequence \
            ORDER BY sequence ASC \
            LIMIT 1 \
        ) nxt ON TRUE \
        WHERE l.sequence = $1",
    )
    .bind(sequence)
    .fetch_optional(pool)
    .await
}

// ---------------------------------------------------------------------------
// Embedded transactions query — statement B of canonical SQL 05
// ---------------------------------------------------------------------------

/// Fetch one page of transactions belonging to a single ledger.
///
/// Partition pruning is total: `t.created_at = $closed_at` is full
/// equality (every transaction in a ledger shares the ledger's exact
/// `closed_at`), so only one monthly partition is touched. Cursor is
/// `(created_at, id) DESC` reusing the `TsIdCursor` codec — same
/// convention as the top-level `GET /v1/transactions`. Caller passes
/// `limit + 1` for `has_more` detection.
pub async fn fetch_transactions(
    pool: &PgPool,
    ledger_sequence: i64,
    closed_at: DateTime<Utc>,
    cursor: Option<&TsIdCursor>,
    limit: i64,
) -> Result<Vec<LedgerTxRow>, sqlx::Error> {
    let cursor_ts = cursor.map(|c| c.ts);
    let cursor_id = cursor.map(|c| c.id);

    sqlx::query_as::<_, LedgerTxRow>(
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
            COALESCE(ops.operation_types, ARRAY[]::text[]) AS operation_types, \
            COALESCE(ctr.contract_ids,    ARRAY[]::text[]) AS contract_ids, \
            t.created_at \
        FROM transactions t \
        JOIN accounts a ON a.id = t.source_id \
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
        ) ctr ON TRUE \
        WHERE t.ledger_sequence = $1 \
          AND t.created_at      = $2 \
          AND ($3::timestamptz IS NULL OR (t.created_at, t.id) < ($3, $4)) \
        ORDER BY t.created_at DESC, t.id DESC \
        LIMIT $5",
    )
    .bind(ledger_sequence)
    .bind(closed_at)
    .bind(cursor_ts)
    .bind(cursor_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}
