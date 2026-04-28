-- Endpoint:     GET /network/stats
-- Purpose:      Top-level chain summary for the home dashboard:
--               latest ledger sequence + close-time, TPS over a 60s window,
--               total accounts, total contracts, plus the wall-clock time
--               the SELECT was executed (for cache-aware freshness on the
--               client). Cacheable with a short TTL (5–15 s).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.2 + §7
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:       (none)
-- Indexes:      ledgers PK, ledgers.idx_ledgers_closed_at,
--               pg_class catalog (for reltuples).
-- Notes:
--   • `latest_ledger_closed_at` powers the §7 "polling indicator — when
--     data was last refreshed" UI element. It is the close time of the
--     newest ledger we've ingested, which is the freshest signal we have
--     about chain-tip lag (matches backend-overview.md §9.1 freshness
--     indicator).
--   • `total_accounts` and `total_contracts` use the planner's reltuples
--     estimate from pg_class instead of a literal `COUNT(*)`. On an explorer
--     DB at chain scale (10s of millions of accounts) an exact count is a
--     full heap scan and would dominate this hot dashboard query. The
--     estimate is refreshed by autovacuum / ANALYZE and is well within the
--     accuracy a "total accounts" UI cell requires. If exact is ever needed,
--     spawn a periodic counter table — do NOT add COUNT(*) here.
--   • TPS is `SUM(transaction_count) / window_seconds` over the closed
--     ledgers in the trailing 60s, computed from the actual span between
--     min/max closed_at in the window (so partial windows and single-ledger
--     windows still yield a stable number, falling back to 0 via NULLIF).
--     `::float8` is used (not `::numeric`) — TPS is a 0–1000 display metric
--     with FE-side rounding, f64 has 14-digit headroom, and surfacing as
--     `float8` keeps the API decode path native (no `rust_decimal` dep).
--   • `latest_ledger_sequence` and `latest_ledger_closed_at` come from a
--     shared LATERAL on the newest ledger row so the planner uses
--     idx_ledgers_closed_at exactly once, not twice.
--   • The remaining sub-selects are independent and run in parallel under
--     the planner's executor; the whole statement is one round-trip.
--   • `generated_at` is `NOW()` evaluated at SELECT time. The API caches
--     the assembled response in-process (~30 s TTL); cache hits return
--     the original `generated_at`, so the frontend can split two
--     distinct signals without mixing them with cache age:
--       - indexer-health lag = generated_at − latest_ledger_closed_at
--       - data staleness ("info from N seconds ago") = Date.now() − generated_at

SELECT
    latest.sequence                                                                            AS latest_ledger_sequence,
    latest.closed_at                                                                           AS latest_ledger_closed_at,
    NOW()                                                                                      AS generated_at,
    (
        SELECT COALESCE(
            SUM(transaction_count)::float8
                / NULLIF(EXTRACT(EPOCH FROM (MAX(closed_at) - MIN(closed_at))), 0),
            0
        )::float8
        FROM ledgers
        WHERE closed_at >= NOW() - INTERVAL '60 seconds'
    )                                                                                          AS tps_60s,
    (SELECT reltuples::bigint FROM pg_class WHERE oid = 'public.accounts'::regclass)           AS total_accounts,
    (SELECT reltuples::bigint FROM pg_class WHERE oid = 'public.soroban_contracts'::regclass)  AS total_contracts
FROM (
    SELECT sequence, closed_at
    FROM ledgers
    ORDER BY closed_at DESC
    LIMIT 1
) latest;
