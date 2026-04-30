-- ============================================================================
-- soroban_invocations_appearances — partitioned (per ADR 0034).
-- Columns: contract_id, transaction_id, ledger_sequence, caller_id, amount, created_at
-- ============================================================================
\echo '## soroban_invocations_appearances'

\echo '### I1 — composite FK to transactions valid'
SELECT COUNT(*) AS violations
FROM soroban_invocations_appearances sia
LEFT JOIN transactions t ON t.id = sia.transaction_id AND t.created_at = sia.created_at
WHERE t.id IS NULL;

\echo '### I2 — contract_id FK to soroban_contracts valid'
SELECT COUNT(*) AS violations
FROM soroban_invocations_appearances sia
LEFT JOIN soroban_contracts c ON c.id = sia.contract_id
WHERE c.id IS NULL;

\echo '### I3 — caller_id FK to accounts valid where set'
SELECT COUNT(*) AS violations
FROM soroban_invocations_appearances sia
LEFT JOIN accounts a ON a.id = sia.caller_id
WHERE sia.caller_id IS NOT NULL AND a.id IS NULL;

\echo '### I4 — ledger_sequence matches parent transaction.ledger_sequence'
SELECT COUNT(*) AS violations
FROM soroban_invocations_appearances sia
JOIN transactions t ON t.id = sia.transaction_id AND t.created_at = sia.created_at
WHERE t.ledger_sequence <> sia.ledger_sequence;

\echo '### I5 — amount (folded duplicates) >= 1 when present'
SELECT COUNT(*) AS violations
FROM soroban_invocations_appearances
WHERE amount IS NOT NULL AND amount < 1;

\echo '### I6 — every invoked contract has at least one event appearance OR is a no-event invocation'
-- Sanity: contract invocations typically emit events, but not always (read-only calls).
-- This is informational — counts the ratio for awareness, not a hard violation.
\echo '#### info: invocation rows | events rows | invocations w/o ANY event'
SELECT
    (SELECT COUNT(*) FROM soroban_invocations_appearances) AS invocation_rows,
    (SELECT COUNT(*) FROM soroban_events_appearances) AS event_rows,
    (SELECT COUNT(*) FROM soroban_invocations_appearances sia
     WHERE NOT EXISTS (
         SELECT 1 FROM soroban_events_appearances sea
         WHERE sea.transaction_id = sia.transaction_id
           AND sea.created_at = sia.created_at
           AND sea.contract_id = sia.contract_id
     )) AS invocations_without_events;
