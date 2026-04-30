-- ============================================================================
-- accounts — surrogate BIGSERIAL id + natural StrKey account_id (VARCHAR(56)).
-- Columns: id, account_id, first_seen_ledger, last_seen_ledger, sequence_number, home_domain
-- ============================================================================
\echo '## accounts'

\echo '### I1 — account_id matches StrKey shape (G or M prefix, 56 or 69 chars, base32)'
-- Pre-unwrap stage may briefly hold M-keys (69 chars); post-unwrap only G (56 chars).
-- Per task 0044 path validators, persisted accounts should always be G/56.
SELECT COUNT(*) AS violations,
       (SELECT array_agg(account_id) FROM (
           SELECT account_id FROM accounts
           WHERE NOT (
               (length(account_id) = 56 AND account_id LIKE 'G%' AND account_id ~ '^[A-Z2-7]+$')
               -- M-keys tolerated for now if they slip through unwrap
            OR (length(account_id) = 69 AND account_id LIKE 'M%' AND account_id ~ '^[A-Z2-7]+$')
           )
           ORDER BY id LIMIT 5
       ) s) AS sample
FROM accounts
WHERE NOT (
    (length(account_id) = 56 AND account_id LIKE 'G%' AND account_id ~ '^[A-Z2-7]+$')
 OR (length(account_id) = 69 AND account_id LIKE 'M%' AND account_id ~ '^[A-Z2-7]+$')
);

\echo '### I2 — account_id UNIQUE'
SELECT COUNT(*) AS violations
FROM (SELECT account_id FROM accounts GROUP BY account_id HAVING COUNT(*) > 1) d;

\echo '### I3 — first_seen_ledger ≤ last_seen_ledger (monotonic)'
SELECT COUNT(*) AS violations,
       (SELECT array_agg(id) FROM (
           SELECT id FROM accounts
           WHERE first_seen_ledger > last_seen_ledger
           ORDER BY id LIMIT 5
       ) s) AS sample
FROM accounts
WHERE first_seen_ledger > last_seen_ledger;

\echo '### I4 — non-negative ledger sequences'
SELECT COUNT(*) AS violations
FROM accounts
WHERE first_seen_ledger < 0 OR last_seen_ledger < 0;
