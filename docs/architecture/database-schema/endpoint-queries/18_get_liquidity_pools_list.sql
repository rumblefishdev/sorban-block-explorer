-- Endpoint:     GET /liquidity-pools
-- Purpose:      Paginated list of liquidity pools with their latest
--               on-chain state (reserves, total shares, TVL). Optional
--               filters: asset pair, minimum TVL.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.13
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :limit                    INT            page size
--   $2  :cursor_created_at_ledger BIGINT         NULL on first page
--   $3  :cursor_pool_id           BYTEA(32)      NULL on first page
--   $4  :asset_a_code             VARCHAR        NULL = no filter
--   $5  :asset_a_issuer_strkey    VARCHAR(56)    NULL = no filter
--   $6  :asset_b_code             VARCHAR        NULL = no filter
--   $7  :asset_b_issuer_strkey    VARCHAR(56)    NULL = no filter
--   $8  :min_tvl                  NUMERIC(28,7)  NULL = no filter
--   $9  :snapshot_window          INTERVAL       freshness window for the
--                                                latest-snapshot pivot
--                                                (e.g. '1 day'::interval)
-- Indexes:      idx_pools_asset_a / idx_pools_asset_b (asset filters),
--               liquidity_pools PK (pool_id) + the implicit btree on
--                  (created_at_ledger, pool_id) — see INDEX GAP note,
--               idx_lps_pool ON (pool_id, created_at DESC) — for the
--                  latest-snapshot lateral lookup.
-- INDEX GAP: ADR 0037 has no btree on `liquidity_pools.created_at_ledger`.
--            With the chosen ordering `(created_at_ledger DESC, pool_id DESC)`
--            the planner falls back to a heap scan + sort. Pool counts are
--            small relative to operations / transactions tables, so this is
--            tolerable for the explorer's lifetime, but
--            `idx_pools_created_at_ledger ON (created_at_ledger DESC, pool_id DESC)`
--            should be added in task **0132** before the table grows past
--            the "small enough to sort" threshold.
-- Notes:
--   • Default ordering is `(created_at_ledger DESC, pool_id DESC)`: newest
--     pools first, deterministic on tie. We deliberately do NOT order by
--     latest-snapshot TVL — that ordering depends on a freshness-window
--     subquery and produces NULL TVLs for stale pools, which forces an
--     awkward NULLS-LAST cursor (the previous draft of this query had it
--     and it was hard to keep keyset-stable). TVL is still surfaced and
--     filterable; the caller can sort client-side within a page or the
--     endpoint can be expanded with an explicit `?sort=tvl` later.
--   • Latest snapshot per pool is fetched via a LATERAL with
--     LIMIT 1 + recent-window predicate. Pools with no snapshot in the
--     window come back with NULL reserves / TVL — the API surfaces them
--     as "stale". The list is unaffected by snapshot staleness.
--   • Asset-leg filter accepts native (`code IS NULL` / `issuer IS NULL`)
--     by leaving both code and issuer params NULL, OR explicit classic
--     identity (both non-NULL). Mixed (one NULL one not) is undefined —
--     the API validates inputs upstream.
--   • Issuer StrKeys resolve via a CTE with the `accounts.account_id`
--     UNIQUE index, then are surfaced via final joins.

WITH issuer_a AS (
    SELECT id FROM accounts WHERE $5::varchar IS NOT NULL AND account_id = $5
),
issuer_b AS (
    SELECT id FROM accounts WHERE $7::varchar IS NOT NULL AND account_id = $7
)
SELECT
    encode(lp.pool_id, 'hex')           AS pool_id_hex,
    asset_type_name(lp.asset_a_type)    AS asset_a_type_name,
    lp.asset_a_type                     AS asset_a_type,
    lp.asset_a_code,
    iss_a.account_id                    AS asset_a_issuer,
    asset_type_name(lp.asset_b_type)    AS asset_b_type_name,
    lp.asset_b_type                     AS asset_b_type,
    lp.asset_b_code,
    iss_b.account_id                    AS asset_b_issuer,
    lp.fee_bps,
    -- Frontend §6.13 shows "fee percentage" (e.g. 0.30 %).
    -- DB stores basis points; conversion is here, not on the client.
    (lp.fee_bps::numeric / 100)         AS fee_percent,
    lp.created_at_ledger,
    s.ledger_sequence                   AS latest_snapshot_ledger,
    s.reserve_a,
    s.reserve_b,
    s.total_shares,
    s.tvl,
    s.volume,
    s.fee_revenue,
    s.created_at                        AS latest_snapshot_at
FROM liquidity_pools lp
LEFT JOIN accounts iss_a ON iss_a.id = lp.asset_a_issuer_id
LEFT JOIN accounts iss_b ON iss_b.id = lp.asset_b_issuer_id
LEFT JOIN LATERAL (
    SELECT
        lps.ledger_sequence,
        lps.reserve_a,
        lps.reserve_b,
        lps.total_shares,
        lps.tvl,
        lps.volume,
        lps.fee_revenue,
        lps.created_at
    FROM liquidity_pool_snapshots lps
    WHERE lps.pool_id    = lp.pool_id
      AND lps.created_at >= NOW() - $9::interval
    ORDER BY lps.created_at DESC, lps.ledger_sequence DESC
    LIMIT 1
) s ON TRUE
WHERE
    ($2::bigint IS NULL OR (lp.created_at_ledger, lp.pool_id) < ($2, $3))
    AND ($4::varchar IS NULL OR lp.asset_a_code = $4)
    AND ($5::varchar IS NULL OR lp.asset_a_issuer_id = (SELECT id FROM issuer_a))
    AND ($6::varchar IS NULL OR lp.asset_b_code = $6)
    AND ($7::varchar IS NULL OR lp.asset_b_issuer_id = (SELECT id FROM issuer_b))
    AND ($8::numeric IS NULL OR s.tvl >= $8)
ORDER BY lp.created_at_ledger DESC, lp.pool_id DESC
LIMIT $1;
