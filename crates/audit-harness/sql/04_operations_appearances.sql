-- ============================================================================
-- operations_appearances — partitioned by RANGE (created_at). Appearance index.
-- Columns: id, transaction_id, type, source_id, destination_id, contract_id,
--          asset_code, asset_issuer_id, pool_id, amount, ledger_sequence, created_at
-- ============================================================================
\echo '## operations_appearances'

\echo '### I1 — every (transaction_id, created_at) → existing transactions row (composite FK)'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(transaction_id) FROM (
           SELECT oa.transaction_id FROM operations_appearances oa
           LEFT JOIN transactions t ON t.id = oa.transaction_id AND t.created_at = oa.created_at
           WHERE t.id IS NULL ORDER BY oa.transaction_id LIMIT 5
       ) s) AS sample
FROM operations_appearances oa
LEFT JOIN transactions t ON t.id = oa.transaction_id AND t.created_at = oa.created_at
WHERE t.id IS NULL;

\echo '### I2 — source_id FK valid where set'
SELECT COUNT(*) AS violations
FROM operations_appearances oa
LEFT JOIN accounts a ON a.id = oa.source_id
WHERE oa.source_id IS NOT NULL AND a.id IS NULL;

\echo '### I3 — destination_id FK valid where set'
SELECT COUNT(*) AS violations
FROM operations_appearances oa
LEFT JOIN accounts a ON a.id = oa.destination_id
WHERE oa.destination_id IS NOT NULL AND a.id IS NULL;

\echo '### I4 — asset_issuer_id FK valid where set'
SELECT COUNT(*) AS violations
FROM operations_appearances oa
LEFT JOIN accounts a ON a.id = oa.asset_issuer_id
WHERE oa.asset_issuer_id IS NOT NULL AND a.id IS NULL;

\echo '### I5 — pool_id FK valid where set'
SELECT COUNT(*) AS violations
FROM operations_appearances oa
LEFT JOIN liquidity_pools lp ON lp.pool_id = oa.pool_id
WHERE oa.pool_id IS NOT NULL AND lp.pool_id IS NULL;

\echo '### I6 — amount (folded duplicate count) >= 1 when present'
SELECT COUNT(*) AS violations
FROM operations_appearances
WHERE amount IS NOT NULL AND amount < 1;
