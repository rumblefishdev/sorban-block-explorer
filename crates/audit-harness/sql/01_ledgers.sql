-- ============================================================================
-- ledgers — backbone timeline anchor (unpartitioned).
-- Columns: sequence, hash, closed_at, protocol_version, transaction_count, base_fee
-- ============================================================================
\echo '## ledgers'

\echo '### I1 — sequence contiguous within indexed range'
WITH range AS (SELECT MIN(sequence) lo, MAX(sequence) hi FROM ledgers),
expected AS (SELECT generate_series(lo, hi) seq FROM range),
missing AS (
    SELECT e.seq FROM expected e
    LEFT JOIN ledgers l ON l.sequence = e.seq
    WHERE l.sequence IS NULL
)
SELECT (SELECT COUNT(*) FROM missing) AS violations,
       (SELECT array_agg(seq ORDER BY seq) FROM (SELECT seq FROM missing ORDER BY seq LIMIT 10) s) AS sample;

\echo '### I2 — hash UNIQUE'
SELECT COUNT(*) AS violations
FROM (
    SELECT hash FROM ledgers GROUP BY hash HAVING COUNT(*) > 1
) dup;

\echo '### I3 — closed_at strictly monotonic by sequence'
WITH ordered AS (
    SELECT sequence, closed_at,
           LAG(closed_at) OVER (ORDER BY sequence) AS prev_t
    FROM ledgers
)
SELECT COUNT(*) AS violations,
       (SELECT array_agg(sequence) FROM (
           SELECT sequence FROM ordered
           WHERE prev_t IS NOT NULL AND closed_at <= prev_t
           ORDER BY sequence LIMIT 5
       ) s) AS sample
FROM ordered
WHERE prev_t IS NOT NULL AND closed_at <= prev_t;

\echo '### I4 — non-negative counts'
SELECT COUNT(*) AS violations
FROM ledgers
WHERE transaction_count < 0 OR base_fee < 0;
