-- ============================================================================
-- liquidity_pool_snapshots — partitioned by RANGE (created_at). LP state time-series.
-- Columns: id, pool_id, ledger_sequence, reserve_a, reserve_b, total_shares,
--          tvl, volume, fee_revenue, created_at
-- ============================================================================
\echo '## liquidity_pool_snapshots'

\echo '### I1 — pool_id FK to liquidity_pools valid'
SELECT COUNT(*) AS violations
FROM liquidity_pool_snapshots lps
LEFT JOIN liquidity_pools lp ON lp.pool_id = lps.pool_id
WHERE lp.pool_id IS NULL;

\echo '### I2 — non-negative reserves and shares'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(id) FROM (
           SELECT id FROM liquidity_pool_snapshots
           WHERE reserve_a < 0 OR reserve_b < 0 OR total_shares < 0
           ORDER BY id LIMIT 5
       ) s) AS sample
FROM liquidity_pool_snapshots
WHERE reserve_a < 0 OR reserve_b < 0 OR total_shares < 0;

\echo '### I3 — analytics fields (tvl, volume, fee_revenue) non-negative when set'
-- Per task 0125 these are populated phase-2; NULL is OK
SELECT COUNT(*) AS violations
FROM liquidity_pool_snapshots
WHERE (tvl IS NOT NULL AND tvl < 0)
   OR (volume IS NOT NULL AND volume < 0)
   OR (fee_revenue IS NOT NULL AND fee_revenue < 0);

\echo '### I4 — at most one snapshot per (pool_id, ledger_sequence) — uq_lp_snapshots_pool_ledger'
SELECT COUNT(*) AS violations
FROM (
    SELECT pool_id, ledger_sequence FROM liquidity_pool_snapshots
    GROUP BY 1,2 HAVING COUNT(*) > 1
) d;

\echo '### I5 — ledger_sequence corresponds to existing ledgers row'
SELECT COUNT(*) AS violations
FROM liquidity_pool_snapshots lps
LEFT JOIN ledgers l ON l.sequence = lps.ledger_sequence
WHERE l.sequence IS NULL;
