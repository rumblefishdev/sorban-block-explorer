-- Endpoint:     GET /liquidity-pools/:id/chart
-- Purpose:      Time-bucketed series for a pool: TVL, volume, fee revenue.
--               Used by the LP detail charts (frontend §6.14).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.14
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :pool_id   BYTEA(32)    raw 32-byte pool id
--   $2  :interval  TEXT         '1h' | '1d' | '1w' (validated by the API
--                                against an explicit allowlist before this
--                                SQL is invoked — date_trunc accepts a
--                                broader vocabulary, but the endpoint
--                                contract restricts it)
--   $3  :from      TIMESTAMPTZ  inclusive lower bound
--   $4  :to        TIMESTAMPTZ  exclusive upper bound
-- Indexes:      idx_lps_pool ON (pool_id, created_at DESC).
-- Notes:
--   • date_trunc accepts a text first argument, which lets the API pass
--     the validated interval keyword directly. The text-to-bucket cast
--     is deterministic and does not break partition pruning because the
--     `created_at` predicate is on the raw column.
--   • The allowlist on $2 is the API's responsibility — it MUST be one of
--     '1h' | '1d' | '1w' (or other values that the endpoint is expanded
--     to support in the future). Passing untrusted text would not be a
--     SQL injection because date_trunc is a builtin, but it would let a
--     caller pick `microseconds` and produce a million-row response.
--   • Aggregation policy:
--       — TVL  : last value in bucket (state at close of bucket)
--       — volume / fee_revenue : SUM (cumulative within bucket)
--     Pools that didn't snapshot in a given bucket simply have no row
--     for that bucket — the frontend interpolates if needed.
--   • `idx_lps_pool` covers the leading equality on pool_id and the
--     range on created_at; date_trunc on a non-indexed expression is
--     fine as long as the WHERE shape matches the index.

SELECT
    date_trunc($2, lps.created_at) AS bucket,
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
GROUP BY date_trunc($2, lps.created_at)
ORDER BY bucket ASC;
