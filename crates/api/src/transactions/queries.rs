//! ClickHouse queries for the transactions endpoints.
//!
//! The public response shape intentionally mirrors the PostgreSQL path —
//! the frontend consumes the generated `@rumblefish/api-types`, so the CH
//! path maps `clickhouse::Row` structs back into the same `queries::*Row`
//! types the handler already knows. Notable CH-vs-PG divergences handled
//! here:
//!
//! - **No `transactions.created_at` on CH.** The API timestamp is the
//!   parent ledger `closed_at`, joined in from `ledgers` (ADR 0044 §5.2).
//! - **`transactions.id` is a deterministic hash surrogate**, not a
//!   `BIGSERIAL`. It is a stable, unique tie-break for the global list
//!   keyset `(ledger_sequence, id)` (canonical SQL 02), but it is NOT
//!   apply-order within a ledger — callers that need on-chain order use
//!   `application_order`.
//! - **`operations_appearances` has no `id` surrogate** (PR #175). The
//!   per-op `appearance_id` is the natural-key `application_order`
//!   (canonical SQL 03 statement C).
//! - **`soroban_events` is the full-payload table** (one row per event). The
//!   archive-unavailable fallback groups per (contract, ledger) to emit one
//!   appearance row per contract — the same wire shape as the PG appearance
//!   index (which additionally carries a fold-count column, not surfaced).
//!
//! ### List pagination + the partition-prune / read-in-order guard
//!
//! Canonical SQL 02 (PR #175 amendment) bounds every page to a single
//! `intDiv(ledger_sequence, 500000)` partition. We reproduce that — first
//! page prunes to the latest partition (`intDiv(max(sequence), 500000)`),
//! subsequent pages prune to the cursor's partition. The known cost is that
//! backward pagination across a 500k-ledger partition boundary stops early;
//! acceptable for an explorer (deep cross-partition cursor walks do not
//! occur in the UI).
//!
//! The partition prune alone is NOT enough. A partition is ~1e8 transactions
//! on mainnet, and `transactions FINAL ... ORDER BY ... LIMIT` reads the
//! **whole partition** (FINAL must merge it before the limit applies) —
//! ~118M rows per page, which under frontend polling exhausted the
//! `api_reader` `read_rows` hourly quota (CH `Code: 201`). The no-filter
//! **Statement A** therefore drops FINAL and orders by the table's physical
//! sort key `(ledger_sequence, application_order)`, so CH reads in primary-key
//! order and stops at the limit (~2e5 rows/page; validated). This is safe
//! because `transactions` is append-only and effectively unique on that key,
//! with all projected columns immutable across versions; a Rust-side dedup is
//! the belt-and-braces. The cursor keys on `application_order` for this path
//! (also the correct in-ledger order — the old `id`-hash tie-break did not
//! preserve it). The filtered Statements B/C still key on the
//! `transactions.id` surrogate (they drive off `operations_appearances`).
//!
//! `operation_types` comes from the shared [`ch::fetch_tx_list_aggregates`]
//! keyed on the ≤ `limit + 1` page rows (sourced from `operations_appearances`
//! by primary-key seek). The per-row `contract_ids` array it once also returned
//! was removed (task 0386) — dead field, whole-table `soroban_contracts FINAL`.

use clickhouse::Row;
use serde::Deserialize;

use crate::common::ch::{
    self, millis_to_utc, operation_type_label, resolve_accounts, resolve_contracts,
};
use crate::common::cursor::{Direction, keyset_sql_desc};

use chrono::{DateTime, Utc};

use super::dto::TxListCursor;

// ---------------------------------------------------------------------------
// Internal query-result rows + resolved params (not serialized; the handler
// maps these into the public response DTOs).
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct TxListRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub application_order: i16,
    /// `None` for Variant A `parse_error` transactions whose envelope was
    /// unavailable (lore-0209).
    pub source_account: Option<String>,
    pub fee_charged: i64,
    pub inner_tx_hash: Option<String>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct TxDetailRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub application_order: i16,
    /// `None` for Variant A `parse_error` transactions whose envelope was
    /// unavailable (lore-0209).
    pub source_account: Option<String>,
    /// The source account's on-ledger `home_domain`, for the SEP-2 federated
    /// address the frontend resolves from it (task 0443, issue #363).
    pub source_account_home_domain: Option<String>,
    pub fee_charged: i64,
    pub inner_tx_hash: Option<String>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub created_at: DateTime<Utc>,
    pub parse_error: bool,
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
    /// Crossed liquidity pools, hex-encoded (full crossed-pool list from
    /// path-payment claim atoms, task 0261/0268).
    pub pool_ids: Vec<String>,
    /// 1-based per-tx apply position (task 0192). `None` for pre-task-0192
    /// rows; the caller falls back to `appearance_id` ordering.
    pub application_order: Option<i16>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct EventAppearanceRow {
    pub contract_id: String,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct InvocationAppearanceRow {
    pub contract_id: String,
    pub caller_account: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

/// Resolved, validated `GET /v1/transactions` list params.
pub struct ResolvedListParams {
    pub limit: i64,
    pub cursor: Option<TxListCursor>,
    pub source_account: Option<String>,
    pub contract_id: Option<String>,
    pub op_type: Option<i16>,
}

// ---------------------------------------------------------------------------
// Row structs (positional decode — SELECT column order MUST match field order)
// ---------------------------------------------------------------------------

/// One page row — slim base columns only. `operation_types` is fetched
/// separately via [`ch::fetch_tx_list_aggregates`] and merged by `id` (CH 26.3
/// cannot compute it inline with a correlated subquery).
#[derive(Debug, Row, Deserialize)]
struct TxPageChRow {
    hash: String,
    ledger_sequence: i64,
    application_order: i16,
    source_account: Option<String>,
    fee_charged: i64,
    inner_tx_hash: Option<String>,
    successful: bool,
    operation_count: i16,
    has_soroban: bool,
    id: i64,
    created_at: i64,
}

impl TxPageChRow {
    /// Merge this page row with its pre-fetched aggregates into the
    /// public `TxListRow`. A tx absent from the aggregate map (no ops /
    /// contracts) gets empty vecs via `unwrap_or_default`.
    fn into_list_row(self, agg: ch::TxListAggregates) -> TxListRow {
        TxListRow {
            id: self.id,
            hash: self.hash,
            ledger_sequence: self.ledger_sequence,
            application_order: self.application_order,
            source_account: self.source_account.filter(|s| !s.is_empty()),
            fee_charged: self.fee_charged,
            inner_tx_hash: self.inner_tx_hash.filter(|s| !s.is_empty()),
            successful: self.successful,
            operation_count: self.operation_count,
            has_soroban: self.has_soroban,
            operation_types: agg.operation_types,
            created_at: millis_to_utc(self.created_at),
        }
    }
}

/// Raw page row for Statement A's two-step path: base columns plus the
/// `source_id` surrogate and `ledger_sequence`, with NO join to `accounts` /
/// `ledgers`. `source_account` + `created_at` are resolved by key-seek in
/// [`resolve_source_and_closed_at`]. The old `LEFT JOIN accounts` + `INNER
/// JOIN ledgers` hash-joins built over the FULL tables (~23M + ~13M) and were
/// the real cost behind the polled list's 35M-rows/page (task 0290) — NOT the
/// partition scan, which reads ~2e5 in primary-key order (`InReverseOrder`).
#[derive(Debug, Row, Deserialize)]
struct TxPageRawRow {
    hash: String,
    ledger_sequence: i64,
    application_order: i16,
    source_id: i64,
    fee_charged: i64,
    inner_tx_hash: Option<String>,
    successful: bool,
    operation_count: i16,
    has_soroban: bool,
    id: i64,
}

#[derive(Debug, Row, Deserialize)]
struct LedgerClosedAtRow {
    sequence: i64,
    closed_at: i64,
}

/// Driver key row for the filtered-list two-step seek (task 0354).
#[derive(Debug, Row, Deserialize)]
struct TxKeyRow {
    ledger_sequence: i64,
    transaction_id: i64,
}

/// Resolve `source_account` + `created_at` for a page of raw rows via key
/// seeks instead of full-table hash joins (task 0290). `accounts WHERE id IN
/// (...)` rides the `idx_acc_id` bloom skip-index (accounts is ORDER BY
/// account_id, so the surrogate `id` is not the sort key — a plain join
/// full-scans ~23M); `ledgers WHERE sequence IN (...)` is a primary-key seek.
/// Both inline `i64` literals (no injection surface — same as
/// `ch::fetch_tx_list_aggregates`). Preserves input order.
async fn resolve_source_and_closed_at(
    client: &clickhouse::Client,
    raw: Vec<TxPageRawRow>,
) -> Result<Vec<TxPageChRow>, clickhouse::error::Error> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let in_list = |vals: &[i64]| {
        vals.iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    let dedup_keys = |f: fn(&TxPageRawRow) -> i64| -> Vec<i64> {
        let mut v: Vec<i64> = raw.iter().map(f).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let ledger_seqs = dedup_keys(|r| r.ledger_sequence);
    // ledgers: sequence -> closed_at (plain MergeTree, PK seek, no FINAL).
    let ledgers_sql = format!(
        "SELECT sequence, closed_at FROM ledgers WHERE sequence IN ({})",
        in_list(&ledger_seqs),
    );

    // Both seeks key off `raw` alone, so they go out together (task 0446).
    // Source StrKeys go through the shared resolver, which sorts, dedups and
    // short-circuits on an empty set itself.
    let (accounts, ledger_rows) = tokio::join!(
        resolve_accounts(client, raw.iter().map(|r| r.source_id).collect()),
        client.query(&ledgers_sql).fetch_all::<LedgerClosedAtRow>(),
    );

    let accounts = accounts?;

    let closed_ats: std::collections::HashMap<i64, i64> = ledger_rows?
        .into_iter()
        .map(|r| (r.sequence, r.closed_at))
        .collect();

    // Build page rows in input order. The old `INNER JOIN ledgers` dropped
    // rows whose ledger row was not present yet; the `ledger_sequence <=
    // max(sequence)` cap in the candidate scan already prevents that, but the
    // `?` here preserves the inner semantics defensively (skip a row missing
    // its ledger).
    Ok(raw
        .into_iter()
        .filter_map(|r| {
            let created_at = *closed_ats.get(&r.ledger_sequence)?;
            Some(TxPageChRow {
                hash: r.hash,
                ledger_sequence: r.ledger_sequence,
                application_order: r.application_order,
                source_account: accounts.get(&r.source_id).cloned(),
                fee_charged: r.fee_charged,
                inner_tx_hash: r.inner_tx_hash,
                successful: r.successful,
                operation_count: r.operation_count,
                has_soroban: r.has_soroban,
                id: r.id,
                created_at,
            })
        })
        .collect())
}

#[derive(Debug, Row, Deserialize)]
struct SurrogateIdRow {
    id: i64,
}

#[derive(Debug, Row, Deserialize)]
struct LedgerSeqRow {
    ledger_sequence: i64,
}

// ---------------------------------------------------------------------------
// Shared projection fragments
// ---------------------------------------------------------------------------

/// Slim per-row projection shared by every list statement: the base list
/// columns plus `t.id` (cursor tie-break / aggregate join key) and
/// `l.closed_at` (derived `created_at`). References only `t.*` / `l.*` — no
/// binds, no correlated subqueries. `operation_types` is fetched in a second,
/// non-correlated pass (`ch::fetch_tx_list_aggregates`) and merged by `id` —
/// CH 26.3 rejects correlated subqueries in SELECT.
///
/// Column order MUST match `TxPageChRow` field order (positional decode).
///
/// EVERY column carries an explicit `AS` alias on purpose. The clickhouse
/// crate validates the result column *names* against the `Row` struct fields.
/// Statements B/C join this projection's `t` to a driver subquery `m` that
/// also has a `ledger_sequence` column, so a bare `t.ledger_sequence` comes
/// back named `t.ledger_sequence` (CH keeps the qualifier to disambiguate) and
/// fails struct decode with "column t.ledger_sequence not found in the struct".
/// Statement A has no such join so the bare form happened to work — aliasing
/// all columns makes the projection robust regardless of the surrounding joins.
const SLIM_PROJECTION: &str = "\
    lower(hex(t.hash)) AS hash, \
    t.ledger_sequence AS ledger_sequence, \
    t.application_order AS application_order, \
    nullIf(src.account_id, '') AS source_account, \
    t.fee_charged AS fee_charged, \
    lower(hex(t.inner_tx_hash)) AS inner_tx_hash, \
    t.successful AS successful, \
    t.operation_count AS operation_count, \
    t.has_soroban AS has_soroban, \
    t.id AS id, \
    l.closed_at AS created_at";

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

pub async fn fetch_list(
    client: &clickhouse::Client,
    params: &ResolvedListParams,
    direction: Direction,
    head: Option<i64>,
) -> Result<Vec<TxListRow>, clickhouse::error::Error> {
    // Resolve StrKey filters to the CH surrogate ids up front. The writer's
    // `cityhash_102_128` surrogate is NOT bit-equivalent to CH's builtin
    // `cityHash64()` (schema header), so the id cannot be computed in SQL —
    // it is looked up against the `accounts` / `soroban_contracts` natural
    // keys. A filter that names a non-existent account/contract matches no
    // rows, so we short-circuit to an empty page.
    // The two filters are validated independently in the handler and neither
    // lookup consumes the other's result, so with BOTH set they go out
    // together (task 0446). The empty-page short-circuit moves after the join:
    // a miss on either filter still yields the same empty page, at the cost of
    // one wasted bounded seek when exactly one of the two names nothing.
    let (source_res, contract_res) = tokio::join!(
        async {
            match params.source_account.as_deref() {
                Some(acct) => resolve_account_surrogate(client, acct).await.map(Some),
                None => Ok(None),
            }
        },
        async {
            match params.contract_id.as_deref() {
                Some(cid) => resolve_contract_surrogate(client, cid).await.map(Some),
                None => Ok(None),
            }
        },
    );
    // Outer Option = "was the filter set", inner = "did it resolve". A set
    // filter that resolves to nothing matches no rows — empty page.
    let source_id: Option<i64> = match source_res? {
        Some(None) => return Ok(Vec::new()),
        Some(Some(id)) => Some(id),
        None => None,
    };
    let contract_surrogate: Option<i64> = match contract_res? {
        Some(None) => return Ok(Vec::new()),
        Some(Some(id)) => Some(id),
        None => None,
    };

    let (op, order) = keyset_sql_desc(direction);
    // Cursor keyset is `(ledger_sequence, id)` (canonical SQL 02). The CH
    // cursor variant carries `ledger_sequence` (partition key + primary sort)
    // and `tiebreak` (the `transactions.id` hash surrogate, within-ledger
    // tie-break). Both are present together or absent together, so the keyset
    // tuple never binds a NULL element. A `Pg`-variant cursor never reaches
    // here — `list_transactions` rejects a cross-datasource cursor with
    // `invalid_cursor` before dispatch — so the `_` arm only ever means
    // "first page".
    let (cursor_ledger, cursor_tiebreak): (Option<i64>, Option<i64>) = match params.cursor.as_ref()
    {
        Some(TxListCursor::Ch {
            ledger_sequence,
            tiebreak,
        }) => (Some(*ledger_sequence), Some(*tiebreak)),
        _ => (None, None),
    };

    // Inline the integer params directly into the filtered-statement SQL rather
    // than `.bind()`-ing them. The clickhouse 0.15 bound-parameter path
    // produced empty results for Statements B/C in production — the
    // literal-equivalent query (validated on prod CH) returns the correct page,
    // the bound form returned none. All values are `i64` / `i16` / `None`→`NULL`,
    // so inlining carries no injection surface (same approach as
    // `common::ch::fetch_tx_list_aggregates`, which already inlines its keys).
    let cl = cursor_ledger.map_or_else(|| "NULL".to_string(), |v| v.to_string());
    let ct = cursor_tiebreak.map_or_else(|| "NULL".to_string(), |v| v.to_string());
    let src = source_id.map_or_else(|| "NULL".to_string(), |v| v.to_string());
    let lim_over = params.limit * 4;
    let lim_peek = params.limit + 1;

    // Head substitution (task 0292 §5/6). On the live first page (`cl` IS NULL)
    // the partition prune and the `<= head` cap below otherwise each re-derive
    // the head with a `(SELECT max(sequence) FROM ledgers)` subquery — work the
    // caller has *already* done via `common::head` (the value compared for the
    // 304 short-circuit). When that head is known we inline it as a literal:
    // fewer subqueries in the heavy statement, and the candidate scan is capped
    // at exactly the head the response is ETag'd with (so body == validator,
    // closing the probe-vs-query race on this path). When `head` is `None`
    // (cursored page — the head is irrelevant to the partition) we keep the
    // subquery form. `head` is an `i64`, no injection surface.
    let head_partition = head.map_or_else(
        || "(SELECT intDiv(max(sequence), 500000) FROM ledgers)".to_string(),
        |h| format!("intDiv({h}, 500000)"),
    );
    let head_max = head.map_or_else(
        || "(SELECT max(sequence) FROM ledgers)".to_string(),
        |h| h.to_string(),
    );

    let rows = match (contract_surrogate, params.op_type) {
        // --- Statement B: contract filter (optionally + op_type) -----------
        (Some(cid), op_type_opt) => {
            // Same partition-bounded restructure as Statement C (see there):
            // `transactions t` pruned to a single partition + streamed, the
            // small driver `m` hashed, FINAL dropped (append-only, Rust dedup)
            // — so the join never merges the whole 3.6B-row table.
            //
            // The driver is the 3-arm contract UNION. The
            // `soroban_invocations_appearances` and `soroban_events` arms seek
            // by `contract_id` (their primary-key prefix); the
            // `operations_appearances` arm scans the pruned partition
            // (`contract_id` is not its PK prefix — deferred skip-index
            // follow-up, same as op_type).
            let ot = op_type_opt.map_or_else(|| "NULL".to_string(), |v| v.to_string());
            let arm = |table: &str| {
                format!(
                    "SELECT ledger_sequence, transaction_id FROM {table} \
                     WHERE contract_id = {cid} \
                       AND intDiv(ledger_sequence, 500000) \
                           = ifNull(intDiv({cl}, 500000), {head_partition}) \
                       AND ({cl} IS NULL OR (ledger_sequence, transaction_id) {op} ({cl}, {ct}))"
                )
            };
            // Step 1: the 3-arm contract driver → ≤lim_over (ledger, tx) keys.
            let driver_sql = format!(
                "SELECT DISTINCT ledger_sequence, transaction_id FROM ( \
                    {arm_ops} UNION DISTINCT {arm_inv} UNION DISTINCT {arm_evt} \
                 ) u \
                 WHERE ledger_sequence <= {head_max} \
                 ORDER BY ledger_sequence {order}, transaction_id {order} \
                 LIMIT {lim_over}",
                arm_ops = arm("operations_appearances"),
                arm_inv = arm("soroban_invocations_appearances"),
                arm_evt = arm("soroban_events"),
            );
            let keys = client.query(&driver_sql).fetch_all::<TxKeyRow>().await?;
            if keys.is_empty() {
                Vec::new()
            } else {
                // Step 2: SEEK `transactions` by the driver keys instead of
                // streaming the whole partition as the join's left side (task
                // 0354: CH can't push `t.id = m.transaction_id` into the scan —
                // `id` is not a PK prefix — so the old `FROM (SELECT * FROM
                // transactions WHERE <partition>) t` read the entire ~1e8-row
                // head partition). Same raw projection as Statement A; source +
                // closed_at resolve by key-seek in `resolve_source_and_closed_at`
                // (dropping the whole-`accounts` `LEFT JOIN src`). The seek on
                // `(ledger_sequence, id) IN (keys)` returns exactly the rows the
                // INNER JOIN on the same keys did.
                let in_tuples = keys
                    .iter()
                    .map(|k| format!("({},{})", k.ledger_sequence, k.transaction_id))
                    .collect::<Vec<_>>()
                    .join(",");
                let partitions = keys
                    .iter()
                    .map(|k| k.ledger_sequence / 500_000)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT \
                        lower(hex(t.hash)) AS hash, \
                        t.ledger_sequence AS ledger_sequence, \
                        t.application_order AS application_order, \
                        t.source_id AS source_id, \
                        t.fee_charged AS fee_charged, \
                        lower(hex(t.inner_tx_hash)) AS inner_tx_hash, \
                        t.successful AS successful, \
                        t.operation_count AS operation_count, \
                        t.has_soroban AS has_soroban, \
                        t.id AS id \
                     FROM transactions t \
                     WHERE (t.ledger_sequence, t.id) IN ({in_tuples}) \
                       AND intDiv(t.ledger_sequence, 500000) IN ({partitions}) \
                       AND ({src} IS NULL OR t.source_id = {src}) \
                       AND ({ot} IS NULL OR ( \
                            SELECT count() FROM operations_appearances oa2 \
                            WHERE oa2.transaction_id = t.id \
                              AND oa2.ledger_sequence = t.ledger_sequence \
                              AND oa2.type = {ot} \
                              AND intDiv(oa2.ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000) \
                       ) > 0) \
                     ORDER BY t.ledger_sequence {order}, t.id {order} \
                     LIMIT 1 BY t.id \
                     LIMIT {lim_peek}",
                );
                let raw = client.query(&sql).fetch_all::<TxPageRawRow>().await?;
                resolve_source_and_closed_at(client, raw).await?
            }
        }

        // --- Statement C: op_type filter only ------------------------------
        (None, Some(op_type)) => {
            // Restructured so NEITHER side of the join is a full-table read:
            //
            //  - `transactions t` is pruned to a single partition and is the
            //    STREAMED (left) side; the ≤ `limit*4`-row driver `m` is the
            //    hash side. The previous `... INNER JOIN transactions t FINAL`
            //    had no prune on `t`, so FINAL merged the entire 3.6B-row
            //    table per request — a single op_type page read billions of
            //    rows and exhausted the `read_rows` quota (CH Code: 201). FINAL
            //    is dropped (append-only, immutable columns, Rust-side dedup).
            //  - `m` (driver) scans the pruned partition by `type`, which is
            //    NOT an `operations_appearances` primary-key prefix (~8e7 rows;
            //    bounded, and op_type filtering is user-initiated, not polled).
            //    Making this a seek needs a skip-index on `type` — deferred
            //    follow-up.
            //
            // `LIMIT 1 BY t.id` before the page `LIMIT`: the `accounts` join has
            // no FINAL (a 16M-row FINAL would be ruinous), so un-merged
            // ReplacingMergeTree versions of the source account fan a single
            // transaction into N identical-`id` rows. Here the page `LIMIT` is
            // applied AFTER the join, so without the dedup it fills with copies
            // of the top tx and the page collapses to 1 row (measured rows=4 /
            // distinct_ids=1). `LIMIT 1 BY t.id` collapses the fan-out in SQL
            // before the page cut, so the limit counts distinct transactions and
            // next-page detection stays correct. (Statement A applies its LIMIT
            // inside the pre-join subquery, so it is unaffected.)
            let sql = format!(
                "SELECT {SLIM_PROJECTION} \
                 FROM ( \
                    SELECT * FROM transactions \
                    WHERE intDiv(ledger_sequence, 500000) \
                          = ifNull(intDiv({cl}, 500000), {head_partition}) \
                 ) t \
                 INNER JOIN ( \
                    SELECT DISTINCT ledger_sequence, transaction_id \
                    FROM operations_appearances \
                    WHERE type = {op_type} \
                      AND intDiv(ledger_sequence, 500000) \
                          = ifNull(intDiv({cl}, 500000), {head_partition}) \
                      AND ledger_sequence <= {head_max} \
                      AND ({cl} IS NULL OR (ledger_sequence, transaction_id) {op} ({cl}, {ct})) \
                    ORDER BY ledger_sequence {order}, transaction_id {order} \
                    LIMIT {lim_over} \
                 ) m ON t.id = m.transaction_id AND t.ledger_sequence = m.ledger_sequence \
                 LEFT JOIN accounts src ON src.id = t.source_id \
                 INNER JOIN ledgers l ON l.sequence = t.ledger_sequence \
                 WHERE ({src} IS NULL OR t.source_id = {src}) \
                 ORDER BY t.ledger_sequence {order}, t.id {order} \
                 LIMIT 1 BY t.id \
                 LIMIT {lim_peek}",
            );
            // No outer keyset re-check: the driver subquery already filtered
            // `(ledger_sequence, transaction_id) {op} (cursor)`, and the JOIN
            // binds `t.id = m.transaction_id` / `t.ledger_sequence =
            // m.ledger_sequence`, so every joined row already satisfies it.
            client.query(&sql).fetch_all::<TxPageChRow>().await?
        }

        // --- Statement A: no contract / op_type filter (default path) ------
        (None, None) => {
            // Read-in-order fast path. `transactions` is ORDER BY
            // `(ledger_sequence, application_order)`, so ordering + keying the
            // page on that tuple lets CH read in primary-key order and stop at
            // LIMIT instead of scanning + sorting the whole partition.
            //
            // FINAL is dropped here ON PURPOSE. With FINAL, CH must merge the
            // entire partition before it can apply the limit — measured ~118M
            // rows read per page on the mainnet head partition; without FINAL
            // the same page reads ~2e5. This is the load-bearing fix for the
            // `read_rows` quota blow-up (CH Code: 201) the polled list path
            // caused. It is safe because `transactions` is append-only and
            // effectively unique on `(ledger_sequence, application_order)`
            // (validated: zero net dedup on the live partition), and every
            // projected column is immutable across ReplacingMergeTree versions,
            // so a non-FINAL read returns identical values. Any rare duplicate
            // row is dropped by the `dedup_by_id` pass below.
            //
            // The cursor therefore keys on `application_order` (the physical
            // sort key — also the correct in-ledger apply order, which the old
            // `id`-hash tie-break did NOT preserve), not the `id` surrogate.
            // See `handlers::list_cursor_for`.
            // Cap the candidate scan at the newest ledger actually present in
            // `ledgers`. The indexer can make a transaction visible slightly
            // ahead of its ledger row; without this bound the inner LIMIT picks
            // those head transactions, and the `INNER JOIN ledgers` below then
            // drops the entire page (their `l` row does not exist yet) — the
            // poll returns an empty list even though the feed is healthy. The
            // join is load-bearing (`created_at = l.closed_at`), so the fix is
            // to never page past the ledgers we have rather than to LEFT JOIN.
            // The bound is the PK prefix, so it prunes via the index and is a
            // no-op except at the live head.
            // The `accounts` / `ledgers` projections are NOT joined here. A
            // hash-join over those tables builds the hash side from the WHOLE
            // table (~23M accounts + ~13M ledgers) regardless of the 11-row
            // page — that, not the partition scan, was the 35M rows/page the
            // polled list read (task 0290). Instead project the raw
            // `source_id` + `ledger_sequence` and resolve `source_account` /
            // `created_at` by key-seek in `resolve_source_and_closed_at`
            // (`accounts.id` rides the idx_acc_id bloom; `ledgers.sequence` is
            // a PK seek).
            let sql = format!(
                "SELECT \
                    lower(hex(t.hash)) AS hash, \
                    t.ledger_sequence AS ledger_sequence, \
                    t.application_order AS application_order, \
                    t.source_id AS source_id, \
                    t.fee_charged AS fee_charged, \
                    lower(hex(t.inner_tx_hash)) AS inner_tx_hash, \
                    t.successful AS successful, \
                    t.operation_count AS operation_count, \
                    t.has_soroban AS has_soroban, \
                    t.id AS id \
                 FROM ( \
                    SELECT * FROM transactions \
                    WHERE intDiv(ledger_sequence, 500000) \
                          = ifNull(intDiv({cl}, 500000), {head_partition}) \
                      AND ledger_sequence <= {head_max} \
                      AND ({cl} IS NULL OR (ledger_sequence, toInt64(application_order)) {op} ({cl}, {ct})) \
                      AND ({src} IS NULL OR source_id = {src}) \
                    ORDER BY ledger_sequence {order}, application_order {order} \
                    LIMIT {lim_peek} \
                 ) t \
                 ORDER BY t.ledger_sequence {order}, t.application_order {order}",
            );
            let raw = client.query(&sql).fetch_all::<TxPageRawRow>().await?;
            resolve_source_and_closed_at(client, raw).await?
        }
    };

    // Statement A drops FINAL for the read-in-order fast path (see above), so
    // a re-ingested transaction could in principle surface as two rows with
    // the same `id`. Drop any such duplicate, keeping the first (the rows are
    // already in the requested order). A no-op on the FINAL'd B/C paths and on
    // the live partition (validated zero net dedup), but cheap insurance on
    // ≤ `limit + 1` rows.
    let mut rows = rows;
    let mut seen = std::collections::HashSet::with_capacity(rows.len());
    rows.retain(|r| seen.insert(r.id));

    // Second pass: fetch operation_types for the page's keys (non-correlated
    // derived-table aggregation; CH 26.3 rejects correlated subqueries in
    // SELECT), then merge onto the page rows by tx id.
    let keys: Vec<(i64, i64)> = rows.iter().map(|r| (r.ledger_sequence, r.id)).collect();
    let mut aggregates = ch::fetch_tx_list_aggregates(client, &keys).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let agg = aggregates.remove(&r.id).unwrap_or_default();
            r.into_list_row(agg)
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Detail
// ---------------------------------------------------------------------------

/// Resolve a transaction hash → parent `ledger_sequence`.
///
/// Reads `transaction_hash_index` directly (PK seek on `hash`), mirroring
/// the PG `lookup_hash_index`. Canonical SQL 03 uses the
/// `transaction_hash_dict` Dictionary as the O(1) hot path; that is a
/// CH-only optimization that can be layered on later without changing this
/// signature. `hash → ledger_sequence` is immutable, so no `FINAL` is
/// required on the ReplacingMergeTree index.
pub async fn lookup_hash_ledger(
    client: &clickhouse::Client,
    hash_hex: &str,
) -> Result<Option<i64>, clickhouse::error::Error> {
    let row = client
        .query(
            "SELECT ledger_sequence FROM transaction_hash_index \
             WHERE hash = unhex(?) LIMIT 1",
        )
        .bind(hash_hex)
        .fetch_optional::<LedgerSeqRow>()
        .await?;
    Ok(row.map(|r| r.ledger_sequence))
}

#[derive(Debug, Row, Deserialize)]
struct TxDetailRawRow {
    id: i64,
    hash: String,
    ledger_sequence: i64,
    application_order: i16,
    source_id: i64,
    fee_charged: i64,
    inner_tx_hash: Option<String>,
    successful: bool,
    operation_count: i16,
    has_soroban: bool,
    created_at: i64,
    parse_error: bool,
}

#[derive(Debug, Row, Deserialize)]
struct OpRawRow {
    op_type: i16,
    source_id: Option<i64>,
    destination_id: Option<i64>,
    contract_id: Option<i64>,
    asset_issuer_id: Option<i64>,
    asset_code: Option<String>,
    pool_ids: Vec<String>,
    application_order: i16,
    ledger_sequence: i64,
    created_at: i64,
}

#[derive(Debug, Row, Deserialize)]
struct EventAppearanceRawRow {
    id: i64,
    ledger_sequence: i64,
    created_at: i64,
}

#[derive(Debug, Row, Deserialize)]
struct InvocationAppearanceRawRow {
    contract_surrogate: i64,
    caller_id: Option<i64>,
    ledger_sequence: i64,
    created_at: i64,
}

pub async fn fetch_detail(
    client: &clickhouse::Client,
    hash_hex: &str,
    ledger_sequence: i64,
) -> Result<Option<TxDetailRow>, clickhouse::error::Error> {
    let raw = client
        .query(
            "SELECT \
                t.id AS id, \
                lower(hex(t.hash)) AS hash, \
                t.ledger_sequence, \
                t.application_order, \
                t.source_id, \
                t.fee_charged, \
                lower(hex(t.inner_tx_hash)) AS inner_tx_hash, \
                t.successful, \
                t.operation_count, \
                t.has_soroban, \
                l.closed_at AS created_at, \
                t.parse_error \
             FROM transactions t FINAL \
             INNER JOIN ledgers l ON l.sequence = t.ledger_sequence \
             WHERE t.ledger_sequence = ? \
               AND (t.hash = unhex(?) OR t.inner_tx_hash = unhex(?))",
        )
        .bind(ledger_sequence)
        .bind(hash_hex)
        .bind(hash_hex)
        .fetch_optional::<TxDetailRawRow>()
        .await?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let source = fetch_source_account(client, raw.source_id).await?;
    Ok(Some(TxDetailRow {
        id: raw.id,
        hash: raw.hash,
        ledger_sequence: raw.ledger_sequence,
        application_order: raw.application_order,
        source_account: source
            .as_ref()
            .map(|s| s.account_id.clone())
            .filter(|s| !s.is_empty()),
        source_account_home_domain: source.and_then(|s| s.home_domain).filter(|s| !s.is_empty()),
        fee_charged: raw.fee_charged,
        inner_tx_hash: raw.inner_tx_hash.filter(|s| !s.is_empty()),
        successful: raw.successful,
        operation_count: raw.operation_count,
        has_soroban: raw.has_soroban,
        created_at: millis_to_utc(raw.created_at),
        parse_error: raw.parse_error,
    }))
}

#[derive(Debug, Row, Deserialize)]
struct SourceAccountRow {
    account_id: String,
    home_domain: Option<String>,
}

/// Source account StrKey + its `home_domain`, in the one seek the detail path
/// already paid for.
///
/// Not `resolve_accounts`: that helper dedups ReplacingMergeTree versions with
/// `LIMIT 1 BY id`, which is exact only for columns that never change across
/// versions. `account_id` is such a column; `home_domain` is not — an account
/// can set, change or clear it — so it needs `argMax` over the table's version
/// column (`ReplacingMergeTree(last_seen_ledger)`), or an arbitrary older
/// domain could be served.
async fn fetch_source_account(
    client: &clickhouse::Client,
    source_id: i64,
) -> Result<Option<SourceAccountRow>, clickhouse::error::Error> {
    client
        .query(
            "SELECT any(account_id) AS account_id, \
                    argMax(home_domain, last_seen_ledger) AS home_domain \
             FROM accounts WHERE id = ? GROUP BY id",
        )
        .bind(source_id)
        .fetch_optional::<SourceAccountRow>()
        .await
}

pub async fn fetch_operations(
    client: &clickhouse::Client,
    transaction_id: i64,
    ledger_sequence: i64,
) -> Result<Vec<OpRow>, clickhouse::error::Error> {
    let raw = client
        .query(
            "SELECT \
                oa.type AS op_type, \
                oa.source_id, \
                oa.destination_id, \
                oa.contract_id, \
                oa.asset_issuer_id, \
                nullIf(oa.asset_code, '') AS asset_code, \
                arrayMap(x -> lower(hex(x)), oa.pool_ids) AS pool_ids, \
                oa.application_order, \
                oa.ledger_sequence, \
                l.closed_at AS created_at \
             FROM operations_appearances oa FINAL \
             /* ledgers l FINAL: ledgers is a ReplacingMergeTree with unmerged \
                duplicate rows. This was correct only because `oa FINAL` \
                propagates FINAL into the join — an implicit CH behavior. Made \
                explicit so dropping `oa FINAL` can't silently double every op. \
                Cheap: the join pins a single sequence. lore-0420 */ \
             INNER JOIN ledgers l FINAL ON l.sequence = oa.ledger_sequence \
             WHERE oa.transaction_id = ? \
               AND oa.ledger_sequence = ? \
               AND intDiv(oa.ledger_sequence, 500000) = intDiv(?, 500000) \
             ORDER BY oa.application_order",
        )
        .bind(transaction_id)
        .bind(ledger_sequence)
        .bind(ledger_sequence)
        .fetch_all::<OpRawRow>()
        .await?;

    let account_ids = raw
        .iter()
        .flat_map(|r| [r.source_id, r.destination_id, r.asset_issuer_id])
        .flatten()
        .collect();
    let contract_ids = raw.iter().filter_map(|r| r.contract_id).collect();
    // Both resolve off `raw` alone — one wave, not two (task 0446).
    let (accounts, contracts) = tokio::join!(
        resolve_accounts(client, account_ids),
        resolve_contracts(client, contract_ids),
    );
    let accounts = accounts?;
    let contracts = contracts?;

    Ok(raw
        .into_iter()
        .map(|r| OpRow {
            // CH `operations_appearances` dropped the BIGSERIAL surrogate
            // (PR #175); `application_order` is the natural per-op key.
            appearance_id: i64::from(r.application_order),
            type_name: operation_type_label(r.op_type),
            op_type: r.op_type,
            source_account: r
                .source_id
                .and_then(|id| accounts.get(&id).cloned())
                .filter(|s| !s.is_empty()),
            destination_account: r
                .destination_id
                .and_then(|id| accounts.get(&id).cloned())
                .filter(|s| !s.is_empty()),
            contract_id: r
                .contract_id
                .and_then(|id| contracts.get(&id).cloned())
                .filter(|s| !s.is_empty()),
            asset_code: r.asset_code.filter(|s| !s.is_empty()),
            asset_issuer: r
                .asset_issuer_id
                .and_then(|id| accounts.get(&id).cloned())
                .filter(|s| !s.is_empty()),
            pool_ids: r.pool_ids,
            application_order: Some(r.application_order),
            ledger_sequence: r.ledger_sequence,
            created_at: millis_to_utc(r.created_at),
        })
        .collect())
}

pub async fn fetch_participants(
    client: &clickhouse::Client,
    transaction_id: i64,
    ledger_sequence: i64,
) -> Result<Vec<String>, clickhouse::error::Error> {
    let raw = client
        .query(
            "SELECT tp.account_id AS id \
             FROM transaction_participants tp FINAL \
             WHERE tp.transaction_id = ? \
               AND tp.ledger_sequence = ? \
               AND intDiv(tp.ledger_sequence, 500000) = intDiv(?, 500000)",
        )
        .bind(transaction_id)
        .bind(ledger_sequence)
        .bind(ledger_sequence)
        .fetch_all::<SurrogateIdRow>()
        .await?;
    let accounts = resolve_accounts(client, raw.iter().map(|r| r.id).collect()).await?;
    // INNER JOIN semantics: drop participants whose account row is absent.
    let mut out: Vec<String> = raw
        .into_iter()
        .filter_map(|r| accounts.get(&r.id).cloned())
        .collect();
    out.sort();
    Ok(out)
}

pub async fn fetch_event_appearances(
    client: &clickhouse::Client,
    transaction_id: i64,
    ledger_sequence: i64,
) -> Result<Vec<EventAppearanceRow>, clickhouse::error::Error> {
    // CH `soroban_events` is the full-payload table (one row per event). We
    // group per (contract, ledger) to produce one appearance row per contract
    // in this tx — the same wire shape as the PG appearance index.
    let raw = client
        .query(
            "SELECT \
                se.contract_id AS id, \
                se.ledger_sequence, \
                any(l.closed_at) AS created_at \
             FROM soroban_events se FINAL \
             JOIN ledgers l ON l.sequence = se.ledger_sequence \
             WHERE se.transaction_id = ? \
               AND se.ledger_sequence = ? \
               AND intDiv(se.ledger_sequence, 500000) = intDiv(?, 500000) \
             GROUP BY se.contract_id, se.ledger_sequence",
        )
        .bind(transaction_id)
        .bind(ledger_sequence)
        .bind(ledger_sequence)
        .fetch_all::<EventAppearanceRawRow>()
        .await?;
    let contracts = resolve_contracts(client, raw.iter().map(|r| r.id).collect()).await?;
    let mut out: Vec<EventAppearanceRow> = raw
        .into_iter()
        .map(|r| EventAppearanceRow {
            contract_id: contracts.get(&r.id).cloned().unwrap_or_default(),
            ledger_sequence: r.ledger_sequence,
            created_at: millis_to_utc(r.created_at),
        })
        .collect();
    // Matches the old `ORDER BY se.ledger_sequence, contract_id` (resolved StrKey).
    out.sort_by(|a, b| {
        (a.ledger_sequence, &a.contract_id).cmp(&(b.ledger_sequence, &b.contract_id))
    });
    Ok(out)
}

pub async fn fetch_invocation_appearances(
    client: &clickhouse::Client,
    transaction_id: i64,
    ledger_sequence: i64,
) -> Result<Vec<InvocationAppearanceRow>, clickhouse::error::Error> {
    let raw = client
        .query(
            "SELECT \
                sia.contract_id AS contract_surrogate, \
                sia.caller_id, \
                sia.ledger_sequence, \
                l.closed_at AS created_at \
             FROM soroban_invocations_appearances sia FINAL \
             /* ledgers l FINAL: defensive dedup — see fetch_operations. Was \
                correct only via `sia FINAL` propagating into the join; made \
                explicit. Single-sequence pin, so cheap. lore-0420 */ \
             INNER JOIN ledgers l FINAL ON l.sequence = sia.ledger_sequence \
             WHERE sia.transaction_id = ? \
               AND sia.ledger_sequence = ? \
               AND intDiv(sia.ledger_sequence, 500000) = intDiv(?, 500000)",
        )
        .bind(transaction_id)
        .bind(ledger_sequence)
        .bind(ledger_sequence)
        .fetch_all::<InvocationAppearanceRawRow>()
        .await?;
    // Both resolve off `raw` alone — one wave, not two (task 0446).
    let (contracts, accounts) = tokio::join!(
        resolve_contracts(client, raw.iter().map(|r| r.contract_surrogate).collect()),
        resolve_accounts(client, raw.iter().filter_map(|r| r.caller_id).collect()),
    );
    let contracts = contracts?;
    let accounts = accounts?;
    let mut out: Vec<InvocationAppearanceRow> = raw
        .into_iter()
        .map(|r| InvocationAppearanceRow {
            contract_id: contracts
                .get(&r.contract_surrogate)
                .cloned()
                .unwrap_or_default(),
            caller_account: r
                .caller_id
                .and_then(|id| accounts.get(&id).cloned())
                .filter(|s| !s.is_empty()),
            ledger_sequence: r.ledger_sequence,
            created_at: millis_to_utc(r.created_at),
        })
        .collect();
    // Matches the old `ORDER BY sia.ledger_sequence, sc.contract_id` (resolved StrKey).
    out.sort_by(|a, b| {
        (a.ledger_sequence, &a.contract_id).cmp(&(b.ledger_sequence, &b.contract_id))
    });
    Ok(out)
}

// ---------------------------------------------------------------------------
// StrKey → surrogate-id resolution
// ---------------------------------------------------------------------------

async fn resolve_account_surrogate(
    client: &clickhouse::Client,
    account_strkey: &str,
) -> Result<Option<i64>, clickhouse::error::Error> {
    // `accounts.id` is deterministic across versions (cityhash of the
    // StrKey), so no FINAL is needed for the id lookup.
    let row = client
        .query("SELECT id FROM accounts WHERE account_id = ? LIMIT 1")
        .bind(account_strkey)
        .fetch_optional::<SurrogateIdRow>()
        .await?;
    Ok(row.map(|r| r.id))
}

async fn resolve_contract_surrogate(
    client: &clickhouse::Client,
    contract_strkey: &str,
) -> Result<Option<i64>, clickhouse::error::Error> {
    let row = client
        .query("SELECT id FROM soroban_contracts WHERE contract_id = ? LIMIT 1")
        .bind(contract_strkey)
        .fetch_optional::<SurrogateIdRow>()
        .await?;
    Ok(row.map(|r| r.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_row_merges_aggregates_and_maps_sentinels() {
        // Slim page row: empty-string sentinels → None, millis → UTC, and the
        // separately-fetched aggregate (op types) merges in by id. Replaces the
        // old correlated-projection mapping.
        let row = TxPageChRow {
            hash: "ab".repeat(32),
            ledger_sequence: 100,
            application_order: 2,
            source_account: Some(String::new()),
            fee_charged: 100,
            inner_tx_hash: None,
            successful: true,
            operation_count: 1,
            has_soroban: false,
            id: 999,
            created_at: 1_700_000_000_000,
        };
        let agg = ch::TxListAggregates {
            operation_types: vec!["CREATE_ACCOUNT".to_string(), "PAYMENT".to_string()],
        };
        let mapped = row.into_list_row(agg);
        assert_eq!(mapped.source_account, None);
        assert_eq!(mapped.inner_tx_hash, None);
        assert_eq!(mapped.id, 999);
        assert_eq!(mapped.ledger_sequence, 100);
        assert_eq!(
            mapped.operation_types,
            vec!["CREATE_ACCOUNT".to_string(), "PAYMENT".to_string()],
        );
        assert_eq!(mapped.created_at, ch::millis_to_utc(1_700_000_000_000));
    }
}
