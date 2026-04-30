-- ============================================================================
-- transaction_participants — partitioned by RANGE (created_at).
-- Columns: transaction_id, account_id, created_at  (composite PK)
-- ============================================================================
\echo '## transaction_participants'

\echo '### I1 — composite FK to transactions valid'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(transaction_id) FROM (
           SELECT tp.transaction_id FROM transaction_participants tp
           LEFT JOIN transactions t ON t.id = tp.transaction_id AND t.created_at = tp.created_at
           WHERE t.id IS NULL ORDER BY tp.transaction_id LIMIT 5
       ) s) AS sample
FROM transaction_participants tp
LEFT JOIN transactions t ON t.id = tp.transaction_id AND t.created_at = tp.created_at
WHERE t.id IS NULL;

\echo '### I2 — account_id FK to accounts valid'
SELECT COUNT(*) AS violations
FROM transaction_participants tp
LEFT JOIN accounts a ON a.id = tp.account_id
WHERE a.id IS NULL;

\echo '### I3 — composite UNIQUE (transaction_id, account_id, created_at) — no duplicate participation'
SELECT COUNT(*) AS violations
FROM (
    SELECT transaction_id, account_id, created_at
    FROM transaction_participants
    GROUP BY 1,2,3 HAVING COUNT(*) > 1
) d;
