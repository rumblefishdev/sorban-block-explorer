-- Endpoint:     GET /liquidity-pools/:id/chart
-- Purpose:      Time-bucketed series for a pool: TVL, volume, fee revenue.
--               Used by the LP detail charts (frontend §6.14).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.14
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :pool_id   BYTEA(32)    raw 32-byte pool id
--   $2  :interval  TEXT         '1h' | '1d' | '1w' — the user-facing
--                                vocabulary, validated by the API against
--                                an explicit allowlist before the SQL is
--                                invoked. The query maps it to a Postgres
--                                `date_trunc` keyword via an inline CASE.
--   $3  :from      TIMESTAMPTZ  inclusive lower bound
--   $4  :to        TIMESTAMPTZ  exclusive upper bound
-- Indexes:      idx_lps_pool ON (pool_id, created_at DESC).
-- Notes:
--   • The user-facing interval vocabulary `'1h' | '1d' | '1w'` is the
--     endpoint contract; `date_trunc` itself wants `'hour' | 'day' | 'week'`.
--     We map between the two with a single CASE expression at query time
--     (cheap; the result is constant across all rows for a given call).
--     The API's allowlist validation MUST stay in place — passing
--     untrusted text into the CASE's `ELSE` branch would error at runtime
--     and is a defensive guarantee against accidental sub-second buckets.
--   • Aggregation policy:
--       — TVL  : last value in bucket (state at close of bucket)
--       — volume / fee_revenue : SUM (cumulative within bucket)
--     Pools that didn't snapshot in a given bucket simply have no row
--     for that bucket — the frontend interpolates if needed.
--   • `idx_lps_pool` covers the leading equality on pool_id and the
--     range on created_at; the CASE-derived bucket expression is a
--     constant within one call, not a per-row-evaluated function on the
--     indexed column, so it doesn't break index usability.

WITH bucket_keyword AS (
    SELECT CASE $2
        WHEN '1h' THEN 'hour'
        WHEN '1d' THEN 'day'
        WHEN '1w' THEN 'week'
        -- No ELSE branch on purpose: the API-side allowlist (`1h | 1d | 1w`)
        -- gates $2 before this SQL runs. If a bad value somehow bypasses the
        -- allowlist, `kw` becomes NULL and `date_trunc(NULL, ts)` returns
        -- NULL — every row groups into a single NULL bucket. That is silent
        -- garbage, NOT a loud parse error (the previous comment claimed
        -- "fail loudly" — that was incorrect). The Rust caller adds a
        -- `debug_assert!` on the interval string to catch allowlist drift
        -- in tests; the handler-side allowlist remains the authoritative
        -- validator at runtime.
    END AS kw
)
SELECT
    date_trunc((SELECT kw FROM bucket_keyword), lps.created_at) AS bucket,
    -- "TVL at close of bucket" via the LAST tvl in time order.
    (
        ARRAY_AGG(lps.tvl ORDER BY lps.created_at DESC, lps.ledger_sequence DESC)
    )[1]                            AS tvl,
    SUM(lps.volume)                 AS volume,
    SUM(lps.fee_revenue)            AS fee_revenue,
    COUNT(*)                        AS samples_in_bucket
FROM liquidity_pool_snapshots lps
WHERE lps.pool_id     = $1
  AND lps.created_at >= $3
  AND lps.created_at <  $4
GROUP BY date_trunc((SELECT kw FROM bucket_keyword), lps.created_at)
ORDER BY bucket ASC;
