-- Endpoint:     GET /liquidity-pools/:id/participants
-- Purpose:      Paginated list of liquidity providers in a pool, ordered by
--               share size descending. Powers the "Pool participants" table
--               on the LP detail page (frontend §6.14).
-- Source:       frontend-overview.md §6.14 (UI element) — added to
--               backend-overview.md §6.2 in this PR; previously absent
--               from the API surface (doc drift caught by the §6.14 vs §6.2
--               cross-check during task 0167).
-- Schema:       ADR 0037 §16 (`lp_positions`)
-- Data sources: DB-only.
-- Inputs:
--   $1  :pool_id              BYTEA(32)    raw 32-byte pool id
--   $2  :limit                INT          page size
--   $3  :cursor_shares        NUMERIC(28,7)  NULL on first page
--   $4  :cursor_account_id    BIGINT       NULL on first page
-- Indexes:      idx_lpp_shares ON lp_positions (pool_id, shares DESC) WHERE shares > 0,
--               accounts PK (id) for StrKey join.
-- Notes:
--   • Keyset on (shares DESC, account_id DESC). The partial index
--     `idx_lpp_shares` is keyed exactly on `(pool_id =, shares DESC)` and
--     is partial `WHERE shares > 0`, so we KEEP `lpp.shares > 0` in the
--     WHERE shape to remain index-eligible. Zero-share rows are
--     historical residue (post-withdrawal positions); they don't belong
--     in a "current participants" view.
--   • `share_percentage` is computed against the pool's latest
--     `total_shares` snapshot. We pick the latest snapshot via a
--     correlated LATERAL with a freshness window — the same pattern as
--     file 19. If no snapshot in the window, the percentage is NULL and
--     the API surfaces it as "—" (consistent with stale-pool handling
--     in the list endpoint, file 18).
--   • The percentage divisor is fetched ONCE per page (not per row): we
--     hoist the latest-snapshot lookup into a CTE keyed by pool_id, then
--     join it to every position row. One snapshot lookup, N position rows.
--   • `first_deposit_ledger` and `last_updated_ledger` give the API
--     enough to display "joined N ago" / "last activity N ago" if the
--     UI design needs it (the §6.14 spec doesn't require it explicitly,
--     but the columns are free since the row is already being read).

WITH latest_snap AS (
    SELECT
        lps.total_shares
    FROM liquidity_pool_snapshots lps
    WHERE lps.pool_id    = $1
      AND lps.created_at >= NOW() - INTERVAL '7 days'
    ORDER BY lps.created_at DESC, lps.ledger_sequence DESC
    LIMIT 1
)
SELECT
    acc.account_id                                         AS account,
    lpp.shares,
    CASE
        WHEN snap.total_shares IS NULL OR snap.total_shares = 0 THEN NULL
        ELSE (lpp.shares * 100.0 / snap.total_shares)
    END                                                    AS share_percentage,
    lpp.first_deposit_ledger,
    lpp.last_updated_ledger
FROM lp_positions lpp
JOIN accounts acc                ON acc.id = lpp.account_id
LEFT JOIN latest_snap snap       ON TRUE
WHERE lpp.pool_id = $1
  AND lpp.shares  > 0
  AND ($3::numeric IS NULL
       OR (lpp.shares, lpp.account_id) < ($3, $4))
ORDER BY lpp.shares DESC, lpp.account_id DESC
LIMIT $2;
