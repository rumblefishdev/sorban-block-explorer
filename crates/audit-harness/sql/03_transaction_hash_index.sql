-- ============================================================================
-- transaction_hash_index — unpartitioned bridge for hash → (ledger_seq, created_at).
-- Columns: hash, ledger_sequence, created_at
-- ============================================================================
\echo '## transaction_hash_index'

\echo '### I1 — every hash routes to existing transactions row'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(encode(thi.hash,'hex')) FROM (
           SELECT thi.hash FROM transaction_hash_index thi
           LEFT JOIN transactions t ON t.hash = thi.hash AND t.created_at = thi.created_at
           WHERE t.hash IS NULL ORDER BY thi.ledger_sequence LIMIT 5
       ) s) AS sample
FROM transaction_hash_index thi
LEFT JOIN transactions t ON t.hash = thi.hash AND t.created_at = thi.created_at
WHERE t.hash IS NULL;

\echo '### I2 — every transactions row has matching hash_index entry'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(encode(t.hash,'hex')) FROM (
           SELECT t.hash FROM transactions t
           LEFT JOIN transaction_hash_index thi ON thi.hash = t.hash
           WHERE thi.hash IS NULL ORDER BY t.id LIMIT 5
       ) s) AS sample
FROM transactions t
LEFT JOIN transaction_hash_index thi ON thi.hash = t.hash
WHERE thi.hash IS NULL;

\echo '### I3 — hash UNIQUE'
SELECT COUNT(*) AS violations
FROM (SELECT hash FROM transaction_hash_index GROUP BY hash HAVING COUNT(*) > 1) d;

\echo '### I4 — hash exactly 32 bytes (matches CHECK)'
SELECT COUNT(*) AS violations
FROM transaction_hash_index WHERE octet_length(hash) <> 32;
