//! ClickHouse queries for the ledgers endpoints.
//!
//! The public response shape intentionally mirrors the PostgreSQL path:
//! `GET /v1/ledgers/:sequence` still embeds full `TransactionListItem`
//! rows because the frontend reuses the global transactions table on
//! ledger detail pages. CH does not store `transactions.created_at`, so
//! the ledger `closed_at` value is joined in and used as the API timestamp.
//! For embedded transactions, `TsIdCursor.id` carries `application_order`
//! on the CH path: unlike PostgreSQL's `BIGSERIAL`, CH `transactions.id`
//! is a deterministic hash surrogate and must not define in-ledger order.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use clickhouse::Row;
use serde::Deserialize;

use crate::common::ch::{self, millis_to_utc, resolve_accounts};
use crate::common::cursor::{Direction, SortOrder, TsIdCursor, keyset_sql, keyset_sql_desc};
use crate::transactions::dto::TransactionListItem;

use super::dto::LedgerListItem;

// ---------------------------------------------------------------------------
// Internal query-result rows (not serialized; the handler maps these into the
// public response DTOs).
// ---------------------------------------------------------------------------

/// LATERAL-derived navigation pair. Kept separate from the public DTO
/// because the response type composes this with an embedded paginated
/// list (`transactions`) that does not come from a single SQL row.
#[derive(Debug)]
pub struct LedgerDetailRow {
    pub sequence: i64,
    pub hash: String,
    pub closed_at: DateTime<Utc>,
    pub protocol_version: i32,
    pub transaction_count: i32,
    pub successful_transaction_count: Option<i32>,
    pub base_fee: i64,
    pub prev_sequence: Option<i64>,
    pub next_sequence: Option<i64>,
}

/// DB-side projection of an embedded transaction row. Owned by the ledgers
/// domain (kept separate from `transactions::TxListRow` so the ledger module
/// does not couple to the transaction module's internal query types). Maps to
/// the shared wire `TransactionListItem`, which the ledger-detail response
/// intentionally embeds. `id` is the internal cursor tie-break, not on the DTO.
#[derive(Debug)]
pub struct LedgerTxRow {
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
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Row, Deserialize)]
struct LedgerListRow {
    sequence: i64,
    hash: String,
    closed_at: i64,
    protocol_version: i32,
    transaction_count: i32,
    base_fee: i64,
}

impl From<LedgerListRow> for LedgerListItem {
    fn from(row: LedgerListRow) -> Self {
        Self {
            sequence: row.sequence,
            hash: row.hash,
            closed_at: millis_to_utc(row.closed_at),
            protocol_version: row.protocol_version,
            transaction_count: row.transaction_count,
            // Filled by `attach_successful_counts` once the page is deduped —
            // it is not a column on `ledgers`.
            successful_transaction_count: None,
            base_fee: row.base_fee,
        }
    }
}

/// One `(ledger, successful count)` pair from [`fetch_successful_counts`].
#[derive(Debug, Row, Deserialize)]
struct LedgerSuccessRow {
    ledger_sequence: i64,
    successful_count: u64,
}

#[derive(Debug, Row, Deserialize)]
struct LedgerDetailChRow {
    sequence: i64,
    hash: String,
    closed_at: i64,
    protocol_version: i32,
    transaction_count: i32,
    base_fee: i64,
    prev_sequence: Option<i64>,
    next_sequence: Option<i64>,
}

impl From<LedgerDetailChRow> for LedgerDetailRow {
    fn from(row: LedgerDetailChRow) -> Self {
        Self {
            sequence: row.sequence,
            hash: row.hash,
            closed_at: millis_to_utc(row.closed_at),
            protocol_version: row.protocol_version,
            transaction_count: row.transaction_count,
            successful_transaction_count: None,
            base_fee: row.base_fee,
            prev_sequence: row.prev_sequence,
            next_sequence: row.next_sequence,
        }
    }
}

/// One embedded-transaction page row — slim base columns only.
/// `operation_types` is fetched separately via
/// [`ch::fetch_tx_list_aggregates`] and merged by the surrogate tx id
/// (CH 26.3 cannot compute it inline with a correlated subquery).
#[derive(Debug, Row, Deserialize)]
struct LedgerTxPageChRow {
    /// `transactions.id` hash surrogate — the aggregate join key. NOT the
    /// API cursor id (CH `id` is not apply-order; see `into_ledger_tx_row`).
    tx_surrogate: i64,
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
}

impl LedgerTxPageChRow {
    /// Merge this page row with its pre-fetched aggregates into `LedgerTxRow`.
    ///
    /// `LedgerTxRow.id` carries `application_order` on the CH path (the
    /// `TsIdCursor.id` tie-break for embedded-tx pagination): CH
    /// `transactions.id` is a deterministic hash surrogate and must not
    /// define in-ledger order, so the cursor keys on `application_order`.
    fn into_ledger_tx_row(
        self,
        agg: ch::TxListAggregates,
        source_account: Option<String>,
    ) -> LedgerTxRow {
        LedgerTxRow {
            id: i64::from(self.application_order),
            hash: self.hash,
            ledger_sequence: self.ledger_sequence,
            application_order: self.application_order,
            source_account,
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

pub async fn fetch_list(
    client: &clickhouse::Client,
    limit: i64,
    cursor: Option<&TsIdCursor>,
    sort: SortOrder,
    direction: Direction,
) -> Result<Vec<LedgerListItem>, clickhouse::error::Error> {
    let cursor_sequence = cursor.map(|c| c.id);
    let (op, order) = keyset_sql(sort, direction);

    // `ledgers` is ORDER BY `sequence`. Paginating by `closed_at` (as the PG
    // path does) forces a full ~12M-row scan + sort on every page (measured),
    // and this endpoint is polled, so that scan recurs and eats the
    // `read_rows` quota. `sequence` is monotonic with `closed_at` (a later
    // ledger closes later), so ordering + keying on `sequence` alone yields the
    // identical page in primary-key read-in-order — a handful of rows per page.
    // `sequence` is unique, so it needs no tie-break. The cursor still carries
    // `closed_at` for the wire `ts`, but the keyset no longer reads it.
    //
    // Dedup: `ledgers` is a ReplacingMergeTree and ~12.8M sequences carry 2
    // content-identical rows (22 carry 3) in unmerged parts. Un-deduped, the
    // keyset page returns each sequence twice — doubled rows, and (because the
    // frontend keys table rows by `sequence`) duplicate React keys that break
    // reconciliation and pile up orphaned rows on every re-sort.
    //
    // Deduped by OVER-FETCH + consecutive collapse in Rust, NOT `FINAL` and NOT
    // `LIMIT 1 BY` — both defeat `optimize_read_in_order` on this seek. Measured
    // on the polled first page (lore-0420):
    //
    //   over-fetch (this)   1,349,927 rows    82 MiB    45 ms
    //   LIMIT 1 BY          4,463,222 rows   272 MiB   105 ms
    //   FINAL              25,964,595 rows  1.55 GiB   370 ms   ← whole table
    //
    // Over-fetching costs NOTHING here: the read is granule-bound, so `LIMIT 60`
    // and `LIMIT 20` read the identical 1,349,927 rows. Same approach-B pattern
    // as `assets::dedup_consecutive` (task 0364).
    //
    // Correct because `ORDER BY sequence` IS the primary key, so a sequence's
    // physical versions are contiguous, and the projected columns are identical
    // across them — "keep first" is deterministic.
    let sql = format!(
        "SELECT \
            l.sequence, \
            lower(hex(l.hash)) AS hash, \
            l.closed_at, \
            l.protocol_version, \
            l.transaction_count, \
            l.base_fee \
        FROM ledgers l \
        WHERE ? IS NULL OR l.sequence {op} ? \
        ORDER BY l.sequence {order} \
        LIMIT ?",
    );

    let rows = client
        .query(&sql)
        .bind(cursor_sequence)
        .bind(cursor_sequence)
        .bind(limit * LEDGER_OVERFETCH)
        .fetch_all::<LedgerListRow>()
        .await?;

    let mut items: Vec<LedgerListItem> = dedup_consecutive(rows, limit as usize)
        .into_iter()
        .map(Into::into)
        .collect();
    attach_successful_counts(client, &mut items).await;
    Ok(items)
}

/// Per-ledger successful-transaction counts for the sequences on one page.
///
/// A SECOND query rather than a JOIN or subquery inside [`fetch_list`]: that
/// read is tuned for `optimize_read_in_order` (over-fetch + collapse — see the
/// measurements in its comment), and hanging an aggregate off it puts that plan
/// at risk. Same two-step shape as `ch::fetch_tx_list_aggregates`, and — the
/// part that matters — the same read guard: an explicit key list plus a
/// partition prune.
///
/// `BETWEEN min AND max` would read the same rows TODAY — `ledgers` is
/// contiguous (13,466,469 sequences over a 13,466,469-wide span, measured
/// 2026-08-12), so a page's min/max spans exactly the ledgers on it, and both
/// forms measured 16,384 read_rows. The key list is insurance, not a fix: if
/// `ledgers` ever gains a hole while `transactions` keeps rows inside it —
/// which the write order allows, since `persist::writer` commits
/// `transactions` BEFORE the `ledgers` marker — a straddling page would sweep
/// all of them under `BETWEEN`. Tasks 0243/0386 were quota outages in that
/// shape, and the guard costs nothing.
///
/// Measured on production 2026-08-12, `index_granularity` 8192:
///
/// | page         | read_rows | bytes   | ms |
/// |--------------|-----------|---------|----|
/// | 10 ledgers   | 16,384    | 160 KiB | 6  |
/// | 101 (`MAX_LIMIT` + peek) | 49,152 | 480 KiB | 19 |
///
/// So it is flat only while the page fits two granules; the widest supported
/// page is 3x that, not "the same". At the polled home widget's ~5.5s cadence
/// (~654 requests/hour) 10 ledgers is ~10.7M read_rows/hour per open tab —
/// the binding quota is `read_rows` (2e9/h), not bytes, giving roughly 190
/// concurrent tabs of headroom for this query alone.
///
/// `LIMIT 1 BY` then `countIf` is the house dedup idiom for a
/// `ReplacingMergeTree` — the TPS query in `network::queries` collapses the same
/// way. Not `FINAL` (0420: 19x read amplification), and not `uniqExact`, which
/// builds a per-group hash set (`contracts::queries` measured an OOM at
/// 3.73 GiB on a wide window).
/// The aggregate's read guard, as two inline SQL lists: the page's sequences,
/// and the distinct partitions they fall in (`PARTITION BY intDiv(sequence,
/// 500000)`).
///
/// Inlined rather than bound because both are `i64` — integers carry no
/// injection risk, mirroring `ch::fetch_tx_list_aggregates`. The partitions go
/// through a `BTreeSet` so a 20-ledger page emits one entry, not twenty
/// repeats, and two only when the page straddles a 500k boundary.
fn key_and_partition_lists(sequences: &[i64]) -> (String, String) {
    let keys = sequences
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let partitions = sequences
        .iter()
        .map(|s| s / 500_000)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    (keys, partitions)
}

async fn fetch_successful_counts(
    client: &clickhouse::Client,
    sequences: &[i64],
) -> Result<HashMap<i64, i32>, clickhouse::error::Error> {
    if sequences.is_empty() {
        return Ok(HashMap::new());
    }

    let (seq_list, partitions) = key_and_partition_lists(sequences);

    let sql = format!(
        "SELECT ledger_sequence AS ledger_sequence, \
                countIf(successful) AS successful_count \
         FROM ( \
             SELECT ledger_sequence, application_order, successful \
             FROM transactions \
             WHERE ledger_sequence IN ({seq_list}) \
               AND intDiv(ledger_sequence, 500000) IN ({partitions}) \
             LIMIT 1 BY ledger_sequence, application_order \
         ) \
         GROUP BY ledger_sequence"
    );

    let rows = client.query(&sql).fetch_all::<LedgerSuccessRow>().await?;

    Ok(rows
        .into_iter()
        // `application_order` is `Int16`, so a per-ledger count cannot exceed
        // 65,536 — the cast is exact. Same convention as the other CH `u64`
        // aggregates in this crate.
        .map(|r| (r.ledger_sequence, r.successful_count as i32))
        .collect())
}

/// Fill `successful_transaction_count` on an already-deduped page, in place.
///
/// A ledger with no aggregate row keeps `None`. That is deliberate: rendering a
/// missing aggregate as `0` would claim every transaction in the ledger failed.
///
/// A FAILED aggregate degrades to `None` rather than failing the request. The
/// split is an addition to a page that rendered without it until now, the wire
/// type is already nullable, and the frontend already has a tested branch for
/// "no split available" — losing the whole ledgers list, and with it the polled
/// home widget, over a decorative aggregate would be the wrong trade.
async fn attach_successful_counts(client: &clickhouse::Client, items: &mut [LedgerListItem]) {
    let sequences: Vec<i64> = items.iter().map(|i| i.sequence).collect();

    match fetch_successful_counts(client, &sequences).await {
        Ok(counts) => {
            for item in items.iter_mut() {
                item.successful_transaction_count = counts.get(&item.sequence).copied();
            }
        }
        Err(e) => {
            tracing::warn!("successful-count aggregate failed, serving totals only: {e}");
        }
    }
}

/// Over-fetch factor for [`fetch_list`]. Prod carries at most 3 physical rows
/// per `sequence` (12.8M carry 2, 22 carry 3), so ×3 fills a full page even in
/// the worst observed case. Free: the read is granule-bound, so a wider `LIMIT`
/// reads the same rows (see the note in `fetch_list`). If duplication ever
/// exceeds ×3 the page merely comes back short — the keyset cursor still
/// advances off the last row, so pagination stays correct, never looping.
const LEDGER_OVERFETCH: i64 = 3;

/// Collapse consecutive same-`sequence` rows from the over-fetched page and
/// truncate to `limit`.
///
/// **Requires `raw` to be ordered by `sequence`** — it only compares against the
/// previous row, so non-adjacent duplicates pass through (see
/// `unsorted_input_is_not_deduplicated`, which pins that contract).
///
/// Two separate guarantees back this, easy to conflate:
/// - duplicates are ADJACENT because the query says `ORDER BY sequence`;
/// - the read is CHEAP because `sequence` is the primary key, so ClickHouse
///   serves it read-in-order.
///
/// The primary key buys the cost, not the adjacency. The ordering itself cannot
/// quietly disappear: this page is keyset-paginated, so the next cursor is taken
/// from the LAST row — drop the `ORDER BY` and pagination breaks loudly (wrong
/// cursor, skipped pages) long before deduplication does. That is why a
/// `HashSet` would be the wrong trade here: it would keep collapsing duplicates
/// on unordered input and mask a page that is already broken.
///
/// "Keep first" is deterministic because a duplicate pair is byte-identical in
/// the projected columns.
fn dedup_consecutive(raw: Vec<LedgerListRow>, limit: usize) -> Vec<LedgerListRow> {
    let mut out: Vec<LedgerListRow> = Vec::with_capacity(limit.min(raw.len()));
    for r in raw {
        if out.last().is_none_or(|p| p.sequence != r.sequence) {
            out.push(r);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

pub async fn fetch_by_sequence(
    client: &clickhouse::Client,
    sequence: i64,
) -> Result<Option<LedgerDetailRow>, clickhouse::error::Error> {
    // The count aggregate reads only `sequence`, which is this function's own
    // parameter, so it does not depend on the header read — the two go out
    // together. `join!` and not `try_join!`: the two failures are not
    // equivalent. A failed header means there is no ledger to serve and must
    // propagate; a failed aggregate must degrade to `None`, exactly as on the
    // list path. `try_join!` would collapse both into one early return and
    // reinstate the 500 the nullable field exists to avoid.
    //
    // The aggregate is a second round trip rather than a scalar subquery on
    // the header read because a subquery returns 0 for a ledger with no
    // `transactions` rows, which is indistinguishable from "all of them
    // failed". Concurrent, that costs latency only on a 404, where the
    // aggregate's answer is thrown away.
    // Bound to a `let` because the builder is a temporary: inlined into
    // `join!` it would be dropped while the future still borrows it (E0716).
    let header_query = client
        .query(
            "SELECT \
                l.sequence, \
                lower(hex(l.hash)) AS hash, \
                l.closed_at, \
                l.protocol_version, \
                l.transaction_count, \
                l.base_fee, \
                (SELECT sequence FROM ledgers WHERE sequence < ? ORDER BY sequence DESC LIMIT 1) AS prev_sequence, \
                (SELECT sequence FROM ledgers WHERE sequence > ? ORDER BY sequence ASC LIMIT 1) AS next_sequence \
            FROM ledgers l \
            WHERE l.sequence = ?",
        )
        .bind(sequence)
        .bind(sequence)
        .bind(sequence);

    let just_this_ledger = [sequence];
    let (header, counts) = tokio::join!(
        header_query.fetch_optional::<LedgerDetailChRow>(),
        fetch_successful_counts(client, &just_this_ledger),
    );

    let Some(row) = header? else {
        return Ok(None);
    };

    let mut detail: LedgerDetailRow = row.into();
    detail.successful_transaction_count = match counts {
        Ok(counts) => counts.get(&sequence).copied(),
        Err(e) => {
            tracing::warn!("successful-count aggregate failed for ledger {sequence}: {e}");
            None
        }
    };

    Ok(Some(detail))
}

pub async fn fetch_transactions(
    client: &clickhouse::Client,
    ledger_sequence: i64,
    cursor: Option<&TsIdCursor>,
    limit: i64,
    direction: Direction,
) -> Result<Vec<LedgerTxRow>, clickhouse::error::Error> {
    let cursor_ts_ms = cursor.map(|c| c.ts.timestamp_millis());
    let cursor_application_order = cursor.map(|c| c.id);
    let (op, order) = keyset_sql_desc(direction);

    // Slim page query — base columns only, no correlated subqueries (CH 26.3
    // rejects those). `t.id` (hash surrogate) is selected as the aggregate
    // join key; the API cursor still keys on `application_order`.
    let sql = format!(
        "SELECT \
            t.id AS tx_surrogate, \
            lower(hex(t.hash)) AS hash, \
            t.ledger_sequence, \
            t.application_order, \
            t.source_id AS source_id, \
            t.fee_charged, \
            lower(hex(t.inner_tx_hash)) AS inner_tx_hash, \
            t.successful, \
            t.operation_count, \
            t.has_soroban, \
            l.closed_at AS created_at \
        FROM transactions t FINAL \
        INNER JOIN ledgers l ON l.sequence = t.ledger_sequence \
        WHERE t.ledger_sequence = ? \
          AND intDiv(t.ledger_sequence, 500000) = intDiv(?, 500000) \
          AND (isNull(?) OR (l.closed_at, toInt64(t.application_order)) {op} (fromUnixTimestamp64Milli(ifNull(?, 0)), ifNull(?, 0))) \
        ORDER BY l.closed_at {order}, t.application_order {order} \
        LIMIT ?",
    );

    let page = client
        .query(&sql)
        .bind(ledger_sequence)
        .bind(ledger_sequence)
        .bind(cursor_ts_ms)
        .bind(cursor_ts_ms)
        .bind(cursor_application_order)
        .bind(limit)
        .fetch_all::<LedgerTxPageChRow>()
        .await?;

    // Second pass: aggregate operation_types for the page's (ledger_sequence,
    // transaction_id) keys (non-correlated; CH-26-safe), then merge by the
    // surrogate tx id.
    let keys: Vec<(i64, i64)> = page
        .iter()
        .map(|r| (r.ledger_sequence, r.tx_surrogate))
        .collect();
    // Resolve source StrKeys by surrogate id (bloom seek) instead of a
    // whole-`accounts` `LEFT JOIN … FINAL ON src.id = t.source_id` (task 0354).
    // Both read off `page` alone — one wave, not two (task 0446).
    let (accounts, aggregates) = tokio::join!(
        resolve_accounts(client, page.iter().map(|r| r.source_id).collect()),
        ch::fetch_tx_list_aggregates(client, &keys),
    );
    let accounts = accounts?;
    let mut aggregates = aggregates?;
    Ok(page
        .into_iter()
        .map(|r| {
            let agg = aggregates.remove(&r.tx_surrogate).unwrap_or_default();
            let source_account = accounts
                .get(&r.source_id)
                .cloned()
                .filter(|s| !s.is_empty());
            r.into_ledger_tx_row(agg, source_account)
        })
        .collect())
}

#[cfg(test)]
mod read_guard_tests {
    use super::key_and_partition_lists;

    #[test]
    fn a_page_inside_one_partition_emits_that_partition_once() {
        let (keys, partitions) = key_and_partition_lists(&[63_903_900, 63_903_901, 63_903_902]);

        assert_eq!(keys, "63903900,63903901,63903902");
        assert_eq!(
            partitions, "127",
            "twenty repeats of the same partition would defeat the prune's purpose"
        );
    }

    #[test]
    fn a_page_straddling_a_boundary_emits_both_partitions_in_order() {
        let (_, partitions) = key_and_partition_lists(&[63_999_999, 64_000_000]);

        assert_eq!(partitions, "127,128");
    }

    #[test]
    fn one_ledger_is_the_detail_paths_shape() {
        let (keys, partitions) = key_and_partition_lists(&[63_903_902]);

        assert_eq!(keys, "63903902");
        assert_eq!(partitions, "127");
    }

    #[test]
    fn keys_keep_page_order_and_duplicates_are_the_callers_problem() {
        // `fetch_list` dedups before calling, so this only pins that the key
        // list is a faithful echo — collapsing here would silently mask a
        // caller that stopped deduping.
        let (keys, _) = key_and_partition_lists(&[3, 1, 3]);

        assert_eq!(keys, "3,1,3");
    }
}

#[cfg(test)]
mod dedup_tests {
    use super::{LedgerListRow, dedup_consecutive};

    fn row(sequence: i64) -> LedgerListRow {
        LedgerListRow {
            sequence,
            hash: format!("h{sequence}"),
            closed_at: sequence * 1000,
            protocol_version: 27,
            transaction_count: 1,
            base_fee: 100,
        }
    }

    /// The lore-0420 bug: the RMT hands back each sequence 2–3× and the page
    /// rendered them all, which also collided the frontend's `sequence` row key.
    /// A full page must come back as DISTINCT sequences in the original order.
    #[test]
    fn collapses_duplicate_sequences_and_fills_the_page() {
        // Desc page as the RMT physically returns it: every sequence doubled,
        // one tripled — over-fetched at ×3 (limit 4 → 12 raw rows).
        let raw = [100, 100, 99, 99, 98, 98, 98, 97, 97, 96, 96, 95]
            .into_iter()
            .map(row)
            .collect();

        let out = dedup_consecutive(raw, 4);

        assert_eq!(
            out.iter().map(|r| r.sequence).collect::<Vec<_>>(),
            vec![100, 99, 98, 97],
            "duplicates must collapse and the page must still be full"
        );
    }

    /// Under-fill is survivable: a short page is fine, but it must never emit a
    /// duplicate (that is what broke React reconciliation).
    #[test]
    fn never_emits_duplicates_even_when_underfilled() {
        let raw = [50, 50, 50, 49, 49, 49].into_iter().map(row).collect();

        let out = dedup_consecutive(raw, 4);

        assert_eq!(
            out.iter().map(|r| r.sequence).collect::<Vec<_>>(),
            vec![50, 49]
        );
    }

    /// Pins the precondition rather than hiding it: this collapses ADJACENT
    /// duplicates only, so unordered input passes duplicates straight through.
    ///
    /// That is deliberate. Ordering is not an incidental detail of this page —
    /// the keyset cursor is read off the last row, so losing `ORDER BY sequence`
    /// corrupts pagination itself. A set-based dedup would keep this test green
    /// while the page silently returned rows in the wrong order under a wrong
    /// cursor; failing here is the cheaper outcome.
    #[test]
    fn unsorted_input_is_not_deduplicated() {
        // Same three sequences as the sorted case, interleaved.
        let raw = [100, 99, 100, 99, 98, 98].into_iter().map(row).collect();

        let out = dedup_consecutive(raw, 6);

        assert_eq!(
            out.iter().map(|r| r.sequence).collect::<Vec<_>>(),
            vec![100, 99, 100, 99, 98],
            "only the adjacent 98/98 pair collapses — the interleaved duplicates \
             survive, because ordering is the caller's contract"
        );
    }
}
