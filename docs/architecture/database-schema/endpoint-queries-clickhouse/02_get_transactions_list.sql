-- ============================================================================
-- ⚠️  API-SHAPE CORRECTION (task 0386) — the per-row `contract_ids` array is
--     REMOVED from the transaction-list API entirely. It was PG-parity
--     scaffolding no frontend rendered; computing it forced a whole-table
--     `JOIN soroban_contracts FINAL` (~200k rows/page). The live list response
--     now carries `operation_types` only. The `contract_surrogate_ids` /
--     `contract_ids` projections below are retained as HISTORICAL reference of
--     the old shape (and of the contract *filter* driver, which still exists) —
--     they are NOT in the live response. Supersedes the ops-only note below.
-- ============================================================================
-- ⚠️  CH 26.3 CORRECTION (task 0243) — do NOT copy the correlated-subquery
--     `operation_types` / `contract_ids` projection below verbatim.
--     Those use CORRELATED scalar subqueries (`SELECT groupUniqArray(oa.type)
--     … WHERE oa.transaction_id = t.id`), which ClickHouse 26.3 rejects:
--       Code: 48 NOT_IMPLEMENTED: can't find correlated column …
--     The live read path uses a NON-correlated two-step (validated on prod
--     CH 26.3): fetch the page of tx keys, then aggregate per
--     `(ledger_sequence, transaction_id) IN (…)` with `GROUP BY transaction_id`.
--     Reuse the shared Rust helper
--     `crates/api/src/common/ch.rs::fetch_tx_list_aggregates` for any new
--     transaction-list module instead of the inline projection here.
-- ============================================================================
-- NOTE: the "value moved" / `values` addition (task 0393) was REMOVED on
--     2026-09-04 — the response no longer carries it and the helper runs the
--     op-types aggregate only.
--     NB the table is `asset_id`-leading, so that filter scans the pruned
--     partition (not a seek). Read-path optimisation is an OPEN 0393 follow-up:
--     no index/projection is baked in — the mechanism must come from a concrete
--     load measurement of this endpoint. Amounts are RAW Int128 (client scales;
--     classic/SAC = 7). Not shown in the statements below.
-- ============================================================================
-- ⚠️  CH READ-COST CORRECTION (task 0243) — `contract_ids` is OPS-ONLY in the
--     live read path. The `arrayConcat`/UNION over operations_appearances +
--     soroban_invocations_appearances + soroban_events shown in statements
--     B/C below is NOT what runs. Both soroban_* tables are ORDER BY
--     (contract_id, …), so the per-page `(ledger_sequence, transaction_id) IN
--     (…)` key filter is a PARTITION SCAN on them, not a key seek. In
--     production a single /transactions page read ~1e8 rows and a handful of
--     requests exhausted the api_reader read_rows hourly quota
--     (CH Code: 201 QUOTA_EXCEEDED), 500-ing every CH endpoint.
--     The live helper sources `contract_ids` from operations_appearances ONLY
--     (primary-key seek). PARITY COST: a contract touched solely via a nested
--     sub-invocation or an emitted event (never a root-op contract_id) is not
--     listed; for the vast majority of Soroban tx the invoked contract IS the
--     root-op contract_id, so list-row contract_ids match PG in practice.
--     A cheap full-parity path (skip-index on transaction_id, or a precomputed
--     per-tx contract_ids column) is a deferred follow-up.
-- ============================================================================
-- ⚠️  CH FINAL READ-COST CORRECTION (task 0243) — `... FINAL ... ORDER BY ...
--     LIMIT` over a partition reads the WHOLE partition: FINAL must merge it
--     before the limit applies (~1.2e8 transactions on the mainnet head).
--     Live read path:
--     • Statement A (no filter, polled) DROPS FINAL and orders/keys on the
--       physical sort key `(ledger_sequence, application_order)` so CH reads in
--       primary-key order and stops at the limit (~2e5 rows/page, validated).
--       Cursor tie-break is `application_order` (also the correct in-ledger
--       order — the `id` hash tie-break shown below does NOT preserve it).
--     • Statements B/C (filtered) must NOT join the driver to
--       `transactions t FINAL` unpruned — that merges the whole 3.6B-row table
--       per request (measured; blew the read_rows quota, Code: 201). Prune
--       `transactions` to the driver's partition and stream it (driver = hash
--       side). The `contract_id` driver now SEEKS via the
--       `idx_oa_contract_id` bloom_filter skip-index on `operations_appearances`
--       (task 0333) — a SPARSE contract previously full-scanned the table
--       (box-measured 13.18 M read_rows / 1609 granules → 245 K / 32 granules
--       with the index; this full scan recurred and blew the quota again on
--       2026-06-29, CH Code: 201). The `type` driver (Statement C) still scans
--       the pruned partition (low-cardinality; bloom less effective) — its own
--       skip-index/analysis remains a deferred follow-up.
--     • ledgers (04) + network (01) are ORDER BY `sequence`, NOT `closed_at`;
--       drive their reads off `sequence` (monotonic with closed_at) to stay on
--       the primary key.
--     FINAL is kept only for single-key / per-page key-seek reads (detail,
--       embedded ledger tx, the aggregate helper) where it is cheap.
-- ============================================================================
-- Endpoint:     GET /transactions
-- Purpose:      Paginated list of transactions. Optional filters:
--               source_account, contract_id, operation_type.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.3
-- Schema:       ADR 0044 + PR-#175 hybrid-surrogate amendment.
-- Data sources: DB-only.
-- Inputs:
--   $1  :limit                Int     page size
--   $2  :cursor_ledger        Int64   NULL on first page (drives partition prune)
--   $3  :cursor_tx_id         Int64   NULL on first page
--   $4  :source_account_id    Int64   NULL = no filter (cityhash64 of source StrKey)
--   $5  :contract_id          Int64   NULL = no filter (cityhash64 of C-StrKey)
--   $6  :op_type              Int16   NULL = no filter
--   $7  :latest_partition     Int64   API computes `intDiv(latest_ledger, 500000)`
--                                     and passes as scan-window upper bound;
--                                     first page only — drops the full-table-FINAL
--                                     scan that previously blew CH memory limit.
-- Indexes:      transactions ORDER BY (ledger_sequence, application_order)
--                 + PARTITION BY intDiv(ledger_sequence, 500000).
--               accounts ORDER BY (account_id) — source join by id.
--               operations_appearances, soroban_invocations_appearances,
--                 soroban_events all by (contract_id, …) or
--                 (ledger_sequence, …) — used for `operation_types[]` and
--                 `contract_surrogate_ids[]` projections.
-- CH Engine:    All Replacing — FINAL required.
-- CH Pattern:   3 statements (A no-filter / B contract / C op_type).
--               Partition prune ALWAYS applied — `$2 IS NULL` first-page
--               path uses `$7` (caller-supplied latest_partition) to bound
--               the scan to one partition. Without this, full-table FINAL
--               on 20M+ tx rows blows the 5.6 GB default memory limit.
-- ADR 0044 §:   §4.2 (transactions), §4.3 (operations_appearances et al.),
--               §4.4 (soroban_events — full payload §5.1), §4.5 (accounts).
--   **PR #175 amendment:** memory blowup observed on first-page no-filter
--   path. Fix: caller passes `$7 = intDiv(max_ledger_sequence, 500000)`
--   so we partition-prune from the start. Frontend can derive `$7` from
--   the latest_ledger_sequence in E01's response or cache it for ~5s.
-- Notes:
--   • Partition prune via `intDiv(t.ledger_sequence, 500000) = ifNull($2_part, $7)`
--     where `$2_part = intDiv($2, 500000)` when cursor set, else `$7`.
--     Limits scan to ONE partition (~500k ledgers worst case).
--   • `contract_surrogate_ids[]` projection: PR #175 dropped `assets.id`
--     surrogate but `soroban_contracts.id` (Int64 cityhash64 hub) is still
--     the surrogate FK on operations_appearances / soroban_invocations_appearances /
--     soroban_events. So the projection works as-is — just shape changed.
--   • `operation_types[]` correlated subquery: cheap because (transaction_id,
--     ledger_sequence) is partition-pruned, ~1 sparse-PK granule per row.

-- ============================================================================
-- Statement A — no contract / op_type filter (default path)
-- ============================================================================
-- Wrap in subquery so the LIMIT applies BEFORE the JOIN to `accounts`.
-- Without this, the planner builds a hashtable over both sides of the
-- JOIN (300k+ accounts × 300k+ tx in one partition) and blows the 5.6 GB
-- memory limit. With the subquery, only `$1` (= 50) rows reach the JOIN.
SELECT
    lower(hex(t.hash))                              AS hash_hex,
    t.ledger_sequence,
    t.application_order,
    src.account_id                                  AS source_account,
    t.fee_charged,
    lower(hex(t.inner_tx_hash))                     AS inner_tx_hash_hex,
    t.successful,
    t.operation_count,
    t.has_soroban,
    (
        SELECT groupUniqArray(oa.type)
        FROM operations_appearances oa FINAL
        WHERE oa.transaction_id = t.id
          AND oa.ledger_sequence = t.ledger_sequence
          AND intDiv(oa.ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)
    )                                               AS operation_types,
    -- NOTE: `contract_surrogate_ids[]` projection deliberately omitted from
    -- statement A — 3 correlated FINAL subqueries × 50 rows blew the 5.6 GB
    -- memory limit on full mainnet partitions. Frontend that needs "touched
    -- contracts" badges can call a separate batched endpoint (or use the
    -- single-tx detail E03). Statement B (contract filter) keeps it because
    -- the driver set is already pre-filtered to ~limit-sized rows.
    t.id                                            AS cursor_id
FROM (
    SELECT *
    FROM transactions FINAL
    WHERE intDiv(ledger_sequence, 500000) = ifNull(intDiv($2, 500000), $7)
      AND ($2 IS NULL OR (ledger_sequence, id) < ($2, $3))
      AND ($4 IS NULL OR source_id = $4)
    ORDER BY ledger_sequence DESC, id DESC
    LIMIT $1
) t
JOIN accounts src ON src.id = t.source_id;  -- no FINAL: account_id is deterministic across versions

-- @@ split @@

-- ============================================================================
-- Statement B — contract filter set (with or without op_type)
-- ============================================================================
-- Driven by the 3-table UNION matched on contract_id; partition prune via
-- intDiv on cursor's ledger or caller-supplied $7.
SELECT
    lower(hex(t.hash))                              AS hash_hex,
    t.ledger_sequence,
    t.application_order,
    src.account_id                                  AS source_account,
    t.fee_charged,
    lower(hex(t.inner_tx_hash))                     AS inner_tx_hash_hex,
    t.successful,
    t.operation_count,
    t.has_soroban,
    (
        SELECT groupUniqArray(oa.type)
        FROM operations_appearances oa FINAL
        WHERE oa.transaction_id = t.id AND oa.ledger_sequence = t.ledger_sequence
          AND intDiv(oa.ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)
    )                                               AS operation_types,
    arrayDistinct(arrayConcat(
        (SELECT groupArray(contract_id) FROM operations_appearances FINAL
         WHERE transaction_id = t.id AND ledger_sequence = t.ledger_sequence AND contract_id IS NOT NULL
           AND intDiv(ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)),
        (SELECT groupArray(contract_id) FROM soroban_invocations_appearances FINAL
         WHERE transaction_id = t.id AND ledger_sequence = t.ledger_sequence
           AND intDiv(ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)),
        (SELECT groupArray(contract_id) FROM soroban_events FINAL
         WHERE transaction_id = t.id AND ledger_sequence = t.ledger_sequence
           AND intDiv(ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000))
    ))                                              AS contract_surrogate_ids,
    t.id                                            AS cursor_id
FROM (
    SELECT DISTINCT ledger_sequence, transaction_id
    FROM (
        SELECT ledger_sequence, transaction_id FROM operations_appearances FINAL
        WHERE contract_id = $5
          AND intDiv(ledger_sequence, 500000) = ifNull(intDiv($2, 500000), $7)
          AND ($2 IS NULL OR (ledger_sequence, transaction_id) < ($2, $3))
        UNION DISTINCT
        SELECT ledger_sequence, transaction_id FROM soroban_invocations_appearances FINAL
        WHERE contract_id = $5
          AND intDiv(ledger_sequence, 500000) = ifNull(intDiv($2, 500000), $7)
          AND ($2 IS NULL OR (ledger_sequence, transaction_id) < ($2, $3))
        UNION DISTINCT
        SELECT ledger_sequence, transaction_id FROM soroban_events FINAL
        WHERE contract_id = $5
          AND intDiv(ledger_sequence, 500000) = ifNull(intDiv($2, 500000), $7)
          AND ($2 IS NULL OR (ledger_sequence, transaction_id) < ($2, $3))
    ) u
    ORDER BY ledger_sequence DESC, transaction_id DESC
    LIMIT $1 * 4
) m
JOIN transactions t FINAL ON t.id = m.transaction_id AND t.ledger_sequence = m.ledger_sequence
JOIN accounts src FINAL ON src.id = t.source_id
WHERE
    ($4 IS NULL OR t.source_id = $4)
    AND ($6 IS NULL OR (
        SELECT count() FROM operations_appearances oa2 FINAL
        WHERE oa2.transaction_id = t.id AND oa2.ledger_sequence = t.ledger_sequence AND oa2.type = $6
          AND intDiv(oa2.ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)
    ) > 0)
ORDER BY t.ledger_sequence DESC, t.id DESC
LIMIT $1;

-- @@ split @@

-- ============================================================================
-- Statement C — op_type filter only (no contract filter)
-- ============================================================================
SELECT
    lower(hex(t.hash))                              AS hash_hex,
    t.ledger_sequence,
    t.application_order,
    src.account_id                                  AS source_account,
    t.fee_charged,
    lower(hex(t.inner_tx_hash))                     AS inner_tx_hash_hex,
    t.successful,
    t.operation_count,
    t.has_soroban,
    (
        SELECT groupUniqArray(oa.type)
        FROM operations_appearances oa FINAL
        WHERE oa.transaction_id = t.id AND oa.ledger_sequence = t.ledger_sequence
          AND intDiv(oa.ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)
    )                                               AS operation_types,
    arrayDistinct(arrayConcat(
        (SELECT groupArray(contract_id) FROM operations_appearances FINAL
         WHERE transaction_id = t.id AND ledger_sequence = t.ledger_sequence AND contract_id IS NOT NULL
           AND intDiv(ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)),
        (SELECT groupArray(contract_id) FROM soroban_invocations_appearances FINAL
         WHERE transaction_id = t.id AND ledger_sequence = t.ledger_sequence
           AND intDiv(ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)),
        (SELECT groupArray(contract_id) FROM soroban_events FINAL
         WHERE transaction_id = t.id AND ledger_sequence = t.ledger_sequence
           AND intDiv(ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000))
    ))                                              AS contract_surrogate_ids,
    t.id                                            AS cursor_id
FROM (
    SELECT DISTINCT transaction_id, ledger_sequence
    FROM operations_appearances FINAL
    WHERE type = $6
      AND intDiv(ledger_sequence, 500000) = ifNull(intDiv($2, 500000), $7)
      AND ($2 IS NULL OR (ledger_sequence, transaction_id) < ($2, $3))
    ORDER BY ledger_sequence DESC, transaction_id DESC
    LIMIT $1 * 4
) m
JOIN transactions t FINAL ON t.id = m.transaction_id AND t.ledger_sequence = m.ledger_sequence
JOIN accounts src FINAL ON src.id = t.source_id
WHERE
    ($4 IS NULL OR t.source_id = $4)
    AND ($2 IS NULL OR (t.ledger_sequence, t.id) < ($2, $3))
ORDER BY t.ledger_sequence DESC, t.id DESC
LIMIT $1;
