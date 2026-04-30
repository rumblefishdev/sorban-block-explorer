-- ============================================================================
-- lp_positions — unpartitioned. Per-account share positions.
-- Columns: pool_id, account_id, shares, first_deposit_ledger, last_updated_ledger
-- Partial UNIQUE: idx_lpp_shares (pool_id, shares DESC) WHERE shares > 0
-- ============================================================================
\echo '## lp_positions'

\echo '### I1 — pool_id FK valid'
SELECT COUNT(*) AS violations
FROM lp_positions p
LEFT JOIN liquidity_pools lp ON lp.pool_id = p.pool_id
WHERE lp.pool_id IS NULL;

\echo '### I2 — account_id FK valid'
SELECT COUNT(*) AS violations
FROM lp_positions p
LEFT JOIN accounts a ON a.id = p.account_id
WHERE a.id IS NULL;

\echo '### I3 — shares ≥ 0 (zero shares retained for future-history per task 0162 emerged decision)'
SELECT COUNT(*) AS violations
FROM lp_positions WHERE shares < 0;

\echo '### I4 — first_deposit_ledger ≤ last_updated_ledger (monotonic)'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(account_id) FROM (
           SELECT account_id FROM lp_positions
           WHERE first_deposit_ledger > last_updated_ledger
           ORDER BY pool_id, account_id LIMIT 5
       ) s) AS sample
FROM lp_positions
WHERE first_deposit_ledger > last_updated_ledger;

\echo '### I5 — (pool_id, account_id) UNIQUE (composite PK)'
SELECT COUNT(*) AS violations
FROM (
    SELECT pool_id, account_id FROM lp_positions
    GROUP BY 1,2 HAVING COUNT(*) > 1
) d;

\echo '### I6 — sum of active positions per pool ≈ latest snapshot.total_shares (within stale tolerance)'
-- Soft check: only verify pools with snapshot in 7-day window. Otherwise skip (snapshot stale).
WITH latest_snap AS (
    SELECT DISTINCT ON (pool_id) pool_id, total_shares, created_at
    FROM liquidity_pool_snapshots
    WHERE created_at >= NOW() - INTERVAL '7 days'
    ORDER BY pool_id, created_at DESC, ledger_sequence DESC
),
position_sums AS (
    SELECT pool_id, SUM(shares) AS sum_active_shares
    FROM lp_positions
    WHERE shares > 0
    GROUP BY pool_id
)
SELECT COUNT(*) AS violations,
       (SELECT array_agg(encode(ls.pool_id,'hex')) FROM (
           SELECT ls.pool_id FROM latest_snap ls
           JOIN position_sums ps ON ps.pool_id = ls.pool_id
           WHERE ABS(ls.total_shares - ps.sum_active_shares) > ls.total_shares * 0.001  -- 0.1% tolerance
           ORDER BY ls.pool_id LIMIT 5
       ) s) AS sample
FROM latest_snap ls
JOIN position_sums ps ON ps.pool_id = ls.pool_id
WHERE ABS(ls.total_shares - ps.sum_active_shares) > ls.total_shares * 0.001;
