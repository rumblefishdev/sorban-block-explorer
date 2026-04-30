-- ============================================================================
-- transactions — partitioned by RANGE (created_at).
-- Columns: id, hash, ledger_sequence, application_order, source_id, fee_charged,
--          inner_tx_hash, successful, operation_count, has_soroban, parse_error,
--          created_at
-- ============================================================================
\echo '## transactions'

\echo '### I1 — hash UNIQUE across partitions (uq_transactions_hash_created_at, but hash alone)'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(encode(hash,'hex')) FROM (
           SELECT hash FROM transactions GROUP BY hash HAVING COUNT(*) > 1 LIMIT 5
       ) s) AS sample
FROM (SELECT hash FROM transactions GROUP BY hash HAVING COUNT(*) > 1) d;

\echo '### I2 — operation_count >= COUNT(operations_appearances rows) per tx'
-- Each appearance row represents at least one operation (per ADR 0037 §7 / task 0163
-- the `amount` column is the count of folded duplicates, semantics non-trivial).
-- Conservative invariant: appearance ROW count must not exceed operation_count.
-- A violation means the parser emitted more appearance rows than ops actually existed.
WITH per_tx AS (
    SELECT t.id, t.operation_count AS declared,
           COUNT(oa.transaction_id) AS appearance_rows
    FROM transactions t
    LEFT JOIN operations_appearances oa
      ON oa.transaction_id = t.id AND oa.created_at = t.created_at
    GROUP BY t.id, t.operation_count
    HAVING COUNT(oa.transaction_id) > t.operation_count
)
SELECT COUNT(*) AS violations,
       (SELECT array_agg(id) FROM (SELECT id FROM per_tx ORDER BY id LIMIT 5) s) AS sample
FROM per_tx;

\echo '### I3 — every transaction.ledger_sequence exists in ledgers'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(t.id) FROM (
           SELECT t.id FROM transactions t
           LEFT JOIN ledgers l ON l.sequence = t.ledger_sequence
           WHERE l.sequence IS NULL ORDER BY t.id LIMIT 5
       ) s) AS sample
FROM transactions t
LEFT JOIN ledgers l ON l.sequence = t.ledger_sequence
WHERE l.sequence IS NULL;

\echo '### I4 — source_id FK valid (every source_id → accounts.id)'
SELECT COUNT(*) AS violations
FROM transactions t
LEFT JOIN accounts a ON a.id = t.source_id
WHERE a.id IS NULL;

\echo '### I5 — non-negative numeric fields'
SELECT COUNT(*) AS violations
FROM transactions
WHERE operation_count < 0 OR fee_charged < 0 OR application_order < 0;

\echo '### I6 — inner_tx_hash either NULL or 32 bytes (matches CHECK)'
SELECT COUNT(*) AS violations
FROM transactions
WHERE inner_tx_hash IS NOT NULL AND octet_length(inner_tx_hash) <> 32;
