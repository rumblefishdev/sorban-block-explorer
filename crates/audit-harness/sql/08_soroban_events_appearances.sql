-- ============================================================================
-- soroban_events_appearances — partitioned. Appearance index (per ADR 0033).
-- Columns: contract_id, transaction_id, ledger_sequence, amount, created_at
-- Composite PK: (contract_id, transaction_id, ledger_sequence, created_at)
-- ============================================================================
\echo '## soroban_events_appearances'

\echo '### I1 — composite FK to transactions valid'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(transaction_id) FROM (
           SELECT sea.transaction_id FROM soroban_events_appearances sea
           LEFT JOIN transactions t ON t.id = sea.transaction_id AND t.created_at = sea.created_at
           WHERE t.id IS NULL ORDER BY sea.transaction_id LIMIT 5
       ) s) AS sample
FROM soroban_events_appearances sea
LEFT JOIN transactions t ON t.id = sea.transaction_id AND t.created_at = sea.created_at
WHERE t.id IS NULL;

\echo '### I2 — contract_id FK to soroban_contracts valid'
SELECT COUNT(*) AS violations
FROM soroban_events_appearances sea
LEFT JOIN soroban_contracts c ON c.id = sea.contract_id
WHERE c.id IS NULL;

\echo '### I3 — ledger_sequence matches the parent transaction.ledger_sequence'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(transaction_id) FROM (
           SELECT sea.transaction_id FROM soroban_events_appearances sea
           JOIN transactions t ON t.id = sea.transaction_id AND t.created_at = sea.created_at
           WHERE t.ledger_sequence <> sea.ledger_sequence
           ORDER BY sea.transaction_id LIMIT 5
       ) s) AS sample
FROM soroban_events_appearances sea
JOIN transactions t ON t.id = sea.transaction_id AND t.created_at = sea.created_at
WHERE t.ledger_sequence <> sea.ledger_sequence;

\echo '### I4 — amount (folded duplicates) >= 1 when present'
SELECT COUNT(*) AS violations
FROM soroban_events_appearances
WHERE amount IS NOT NULL AND amount < 1;
