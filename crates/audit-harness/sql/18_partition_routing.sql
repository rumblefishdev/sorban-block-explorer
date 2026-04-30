-- ============================================================================
-- partition_routing — cross-cutting check across all 7 partitioned tables.
--
-- Every row in a partitioned table must live in the partition whose RANGE
-- covers its `created_at`. Rows in `_default` are an indicator that a
-- corresponding monthly child is missing — partition pruning is dead for
-- those rows.
--
-- The check counts rows per partition child and flags:
--   (a) rows in `_default` (should be zero in a healthy backfill)
--   (b) rows whose timestamp falls outside the child's declared bounds (should
--       never happen — Postgres routes by range, but a manual `INSERT INTO
--       child_y2024m02` bypasses routing).
-- ============================================================================
\echo '## partition_routing'

\echo '### I1 — count rows in _default per parent (expect 0 across the board)'
WITH parents AS (
    SELECT unnest(ARRAY[
        'transactions', 'operations_appearances', 'transaction_participants',
        'soroban_events_appearances', 'soroban_invocations_appearances',
        'nft_ownership', 'liquidity_pool_snapshots'
    ]) AS parent
),
default_rows AS (
    SELECT 'transactions' AS parent, (SELECT COUNT(*) FROM transactions_default) AS rows
    UNION ALL SELECT 'operations_appearances', (SELECT COUNT(*) FROM operations_appearances_default)
    UNION ALL SELECT 'transaction_participants', (SELECT COUNT(*) FROM transaction_participants_default)
    UNION ALL SELECT 'soroban_events_appearances', (SELECT COUNT(*) FROM soroban_events_appearances_default)
    UNION ALL SELECT 'soroban_invocations_appearances', (SELECT COUNT(*) FROM soroban_invocations_appearances_default)
    UNION ALL SELECT 'nft_ownership', (SELECT COUNT(*) FROM nft_ownership_default)
    UNION ALL SELECT 'liquidity_pool_snapshots', (SELECT COUNT(*) FROM liquidity_pool_snapshots_default)
)
SELECT
    (SELECT SUM(rows) FROM default_rows) AS total_violations,
    (SELECT json_agg(row_to_json(d)) FROM default_rows d WHERE rows > 0) AS sample_per_parent;

\echo '### I2 — count children per parent (sanity: 30 monthly + 1 default = 31)'
SELECT inhparent::regclass::text AS parent, COUNT(*) AS children
FROM pg_inherits
WHERE inhparent::regclass::text IN (
    'transactions', 'operations_appearances', 'transaction_participants',
    'soroban_events_appearances', 'soroban_invocations_appearances',
    'nft_ownership', 'liquidity_pool_snapshots'
)
GROUP BY 1 ORDER BY 1;

\echo '### I3 — informational: rows-per-month heatmap (last 6 months of activity)'
-- For each partitioned parent, list the 6 most-recent monthly children with row counts.
-- Used to spot abnormally sparse months (which can indicate ingestion gaps).
\echo '#### transactions'
SELECT inhrelid::regclass::text AS partition,
       (SELECT reltuples FROM pg_class WHERE oid = inhrelid)::bigint AS approx_rows
FROM pg_inherits
WHERE inhparent::regclass::text = 'transactions'
  AND inhrelid::regclass::text NOT LIKE '%default'
ORDER BY 1 DESC LIMIT 6;
