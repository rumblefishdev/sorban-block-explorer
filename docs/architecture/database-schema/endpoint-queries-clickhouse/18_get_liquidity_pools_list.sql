-- Endpoint:     GET /liquidity-pools
-- Purpose:      Paginated list of liquidity pools with their latest
--               on-chain state + a compute-at-read USD TVL. Optional
--               filters: asset code / pair, and pool kind. (Minimum-TVL
--               filtering is NOT supported — see Notes.)
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.13
-- Schema:       ADR 0044 + PR-#175 hybrid-surrogate amendment.
-- Data sources: DB-only.
-- Inputs:
--   $1  :limit                          Int     page size
--   $2  :cursor_last_updated_ledger     Int64   NULL on first page
--   $3  :cursor_pool_id                 String  NULL on first page (hex, optional)
--   $4  :asset_code                     String  NULL = no filter; substring of
--                                               a leg's DISPLAYED code, or
--                                               `A/B` for a pair
--   $5  :pool_kind                      Int16   NULL = no filter; 0 classic,
--                                               1 soroban
--   (the former $8 :min_tvl is retired — the API rejects it with 400)
--   (the former per-leg $4..$7 positional filters are retired — task 0374:
--    they named a leg by its position in a pair, which a list of two to four
--    legs has no equivalent for, and no client used them)
-- Indexes:      liquidity_pools ORDER BY (pool_id) — full scan + sort here;
--                 the table is small relative to fact tables.
--               accounts ORDER BY (account_id) — issuer joins by id.
--               liquidity_pool_snapshots ORDER BY (pool_id, ledger_sequence)
--                 + intDiv partition — latest-snapshot lookup via argMax GROUP BY.
--               ledgers ORDER BY (sequence) — closed_at JOIN for display.
-- CH Engine:    liquidity_pools — Replacing(last_updated_ledger) (FINAL).
--               liquidity_pool_snapshots — Replacing partitioned (FINAL).
--               accounts — Replacing (FINAL).
-- CH Pattern:   FINAL on Replacing reads; latest-snapshot via argMax + GROUP BY
--               subquery joined to lp on pool_id; closed_at via JOIN ledgers.
-- ADR 0044 §:   §4.7 (liquidity_pools state Replacing — engine swap per PR
--                 #175 from plain MergeTree to Replacing(last_updated_ledger)),
--                 §4.5 (state Replacing), §4.1 (ledgers MergeTree).
--   **PR #175 amendment:** `liquidity_pools.created_at_ledger` dropped;
--   ordering now uses `last_updated_ledger` (RMT version slot). Pool's
--   true creation ledger is derivable as
--   `MIN(ledger_sequence) FROM liquidity_pool_snapshots GROUP BY pool_id`
--   if needed for display — currently not projected (frontend defaults
--   to "most recently active" ordering, which `last_updated_ledger`
--   naturally provides).
-- Notes:
--   • **task 0199 — USD TVL is computed at read, not projected here.** The
--     snapshot `tvl` / `volume` / `fee_revenue` columns are never written
--     (ADR 0053), so this query no longer reads them. After the page rows
--     come back, the API issues ONE batched lookup against
--     `prices.price_usd_series_1h` for the page's distinct leg identities
--     (last close with `close_usd > 0` within 48 h, in-progress hour
--     excluded — non-positive rows are the prices-side 0171 sentinel and
--     are treated as absent) and computes
--     `tvl = reserve_a·close_a + reserve_b·close_b` per row, NULL unless
--     BOTH legs price. `volume` / `fee_revenue` are detail-endpoint only and
--     serialise as null on the list.
--   • **`filter[min_tvl]` is rejected with HTTP 400.** It used to be a
--     `tvl_pools` pre-filter CTE over the snapshot `tvl` column — which,
--     being unwritten, matched nothing and returned an empty page while the
--     rows themselves now carry real USD TVL. A compute-at-read value cannot
--     filter page membership without TVL for ALL pools per request; that
--     needs the prices-side identity-keyed materialized series. Until then
--     the API says so explicitly rather than answering "no pools".
--   • Cursor ordering is `activity_ledger DESC`, where `activity_ledger` is
--     `greatest(last_updated_ledger, max(pool_state_changes.ledger_sequence))`
--     — "most recently active first", for BOTH kinds.
--     `last_updated_ledger` alone does not mean that: it is the RMT version,
--     bumped on every change to a CLASSIC pool's entry, but written once at
--     registration for a soroban pool, whose activity lives in
--     `pool_state_changes`. Measured 2026-09-09: for 662 of 734 soroban pools
--     the real activity is newer than the column, by 211 days on average.
--     Ordering on the raw column opened the list on just-registered pools —
--     the emptiest end (35% of the first page carried shares, against 75% of
--     the population). `greatest` needs no `pool_kind` branch: a classic pool
--     has no state-change rows so the column wins, and a soroban pool's
--     activity is never earlier than its registration.
--     The outer ORDER BY MUST repeat the paging CTE's expression, or the page
--     holds the right rows in the wrong order and consecutive pages overlap.
--   • argMax over GROUP BY rather than correlated scalar — CH 26.x
--     rejects correlated subqueries with ORDER BY/LIMIT in JOIN.
--   • **Pair filtering is a distinctness condition, not two column tests.**
--     `USDC/XLM` must match two DIFFERENT legs. Over a pair that was one
--     column each; over a list it is Hall's condition for two sets —
--     something matches the first needle, something matches the second, and
--     at least TWO legs match either. Without the last clause a single USDC
--     leg would satisfy `USDC/USDC` on its own.
--     The needle set is `SELECT id FROM assets WHERE position(lower(if(
--     asset_type = 0, 'XLM', toString(asset_code))), lower(?)) > 0` — the
--     `type = 0` arm is load-bearing, since native carries an empty code on
--     the ledger. CH deduplicates the repeated subquery: measured
--     1,218,694 read_rows / 87 ms for the pair form, essentially the same as
--     a single needle.
--   • **Legs, not a pair (task 0374).** `liquidity_pools.legs` is an
--     `Array(Int64)` of asset surrogates in registration order — two for a
--     classic pool, two to four for a Soroban one. It replaced
--     `asset_{a,b}_{type,code,issuer_id}`, which could not express a
--     three-leg stable pool and forced a Soroban row to write placeholder
--     values that read downstream as native XLM.
--     The identities behind those surrogates are NOT joined here: the API
--     collects the page's distinct leg ids and resolves them in ONE batched
--     statement (`common::asset_identity`, shared with the account
--     balance-change rows), which also carries the icon and the observed-SAC
--     flag. The SAC address itself is DERIVED at the response boundary from
--     `(code, issuer, network)`, never looked up (ADR 0051).

SELECT
    lower(hex(lp.pool_id))                                                          AS pool_id_hex,
    toInt16(lp.pool_kind)                                                           AS pool_kind,
    lp.legs                                                                         AS legs,
    lp.fee_bps,
    toDecimal64(lp.fee_bps, 2) / 100                                                AS fee_percent,
    lp.last_updated_ledger                                                          AS last_updated_ledger,
    s.latest_ledger_sequence                                                        AS latest_snapshot_ledger,
    s.reserve_a,
    s.reserve_b,
    s.total_shares,
    -- tvl / volume / fee_revenue are NOT projected from snapshots (task 0199):
    -- those columns are never written. The API adds a compute-at-read USD
    -- `tvl` per row from a batched prices lookup (see Notes); `volume` and
    -- `fee_revenue` stay null on the list — detail-only.
    l_snap.closed_at                                                                AS latest_snapshot_at
FROM liquidity_pools lp FINAL
LEFT JOIN (
    SELECT
        pool_id,
        max(ledger_sequence)                      AS latest_ledger_sequence,
        argMax(reserve_a,        ledger_sequence) AS reserve_a,
        argMax(reserve_b,        ledger_sequence) AS reserve_b,
        argMax(total_shares,     ledger_sequence) AS total_shares
    FROM liquidity_pool_snapshots FINAL
    GROUP BY pool_id
) s ON s.pool_id = lp.pool_id
LEFT JOIN ledgers l_snap ON l_snap.sequence = s.latest_ledger_sequence
WHERE
    ($2 IS NULL OR (activity_ledger, lower(hex(lp.pool_id))) < ($2, $3))
    -- One needle: any leg matches. A pair ($4 = 'A/B') adds the distinctness
    -- clause — see Notes.
    AND ($4 IS NULL OR arrayExists(x -> x IN (
            SELECT id FROM assets
            WHERE position(lower(if(asset_type = 0, 'XLM', toString(asset_code))), lower($4)) > 0
        ), lp.legs))
    AND ($5 IS NULL OR lp.pool_kind = $5)
    -- No min-TVL predicate: `filter[min_tvl]` is rejected with 400 (see Notes).
ORDER BY activity_ledger DESC, lp.pool_id DESC
LIMIT $1;
