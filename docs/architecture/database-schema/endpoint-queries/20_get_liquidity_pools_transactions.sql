-- Endpoint:     GET /liquidity-pools/:id/transactions
-- Purpose:      Paginated transactions touching a given pool — deposits,
--               withdrawals, and trades. Default: most recent first.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.14
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :pool_id             BYTEA(32)    raw 32-byte pool id
--   $2  :limit               INT          page size
--   $3  :cursor_created_at   TIMESTAMPTZ  NULL on first page
--   $4  :cursor_tx_id        BIGINT       NULL on first page
-- Indexes:      idx_ops_app_pool ON (pool_id, created_at DESC) WHERE pool_id IS NOT NULL,
--               transactions PK (id, created_at) — composite-FK join.
-- Notes:
--   • Driver is `operations_appearances` filtered by `pool_id`. The partial
--     index `idx_ops_app_pool` is keyed exactly on this shape, so the
--     planner walks it directly with the cursor predicate.
--   • The WHERE clause keeps `pool_id IS NOT NULL` implicitly (we filter by
--     a concrete pool_id), so the partial index remains usable.
--   • DISTINCT ON deduplicates when one tx has multiple ops touching the
--     same pool (rare but valid — multi-op transactions).
--   • Frontend §6.14 expanded: "Transaction history should distinguish
--     between trade activity and liquidity management activity." We
--     surface the full op-type list per tx so the frontend can categorize
--     each row (LP-mgmt op types: liquidity_pool_deposit /
--     liquidity_pool_withdraw; trade op types: path_payment_*, manage_*,
--     create_passive_sell_offer, etc.). Categorization is policy, not SQL —
--     the frontend owns the "is this a trade" rule.

WITH matched_ops AS (
    -- DISTINCT ON / ORDER BY aligned with newest-first so LIMIT truncates
    -- the tail (oldest matches), not an arbitrary middle. The partial
    -- index `idx_ops_app_pool ON (pool_id, created_at DESC)` is exactly
    -- the descending walk shape we need.
    SELECT DISTINCT ON (oa.created_at, oa.transaction_id)
        oa.transaction_id,
        oa.created_at,
        oa.id AS op_appearance_id
    FROM operations_appearances oa
    WHERE oa.pool_id = $1
      AND ($3::timestamptz IS NULL OR (oa.created_at, oa.transaction_id) < ($3, $4))
    ORDER BY oa.created_at DESC, oa.transaction_id DESC, oa.id
    LIMIT $2 * 4
)
SELECT
    encode(t.hash, 'hex')   AS hash_hex,
    t.ledger_sequence,
    src.account_id          AS source_account,
    t.fee_charged,
    t.successful,
    t.operation_count,
    t.has_soroban,
    ops.operation_types,    -- §6.14: distinguish trade vs LP-mgmt activity
    t.created_at,
    t.id                    AS cursor_tx_id
FROM matched_ops m
JOIN transactions t
       ON t.id         = m.transaction_id
      AND t.created_at = m.created_at
JOIN accounts src ON src.id = t.source_id
LEFT JOIN LATERAL (
    SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
) ops ON TRUE
ORDER BY t.created_at DESC, t.id DESC
LIMIT $2;
