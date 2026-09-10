-- Endpoint:     GET /liquidity-pools/:id
-- Purpose:      Pool detail: identity (legs, kind, fee, last_updated_ledger) +
--               latest on-chain state (reserves, total shares, TVL).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.14
-- Schema:       ADR 0044 + PR-#175 hybrid-surrogate amendment.
-- Data sources: DB-only.
-- Inputs:
--   $1  :pool_id  FixedString(32)  raw 32-byte pool id (or hex via unhex())
-- Indexes:      liquidity_pools ORDER BY (pool_id) — direct PK seek.
--               liquidity_pool_snapshots ORDER BY (pool_id, ledger_sequence)
--                 + PARTITION BY intDiv.
--               accounts ORDER BY (account_id) — issuer joins by id.
--               ledgers ORDER BY (sequence) — closed_at JOIN.
-- CH Engine:    liquidity_pools — Replacing(last_updated_ledger) (FINAL).
--               liquidity_pool_snapshots — Replacing partitioned (FINAL).
--               accounts — Replacing (FINAL).
--               ledgers — MergeTree (no FINAL).
-- CH Pattern:   single-row variant of E18; latest snapshot via argMax over
--                 a pool-pinned scan; closed_at via JOIN ledgers.
-- ADR 0044 §:   §4.7 (liquidity_pools Replacing — PR #175 engine swap),
--                 §4.5 (state Replacing), §4.1 (ledgers), §5.2 (closed_at
--                 via JOIN ledgers).
--   **PR #175 amendment:** `liquidity_pools.created_at_ledger` dropped;
--   use `last_updated_ledger` (RMT version slot). True creation ledger
--   derivable as `MIN(ledger_sequence) FROM liquidity_pool_snapshots
--   WHERE pool_id = $1` — projected explicitly below for display parity.
-- Notes:
--   • `last_updated_ledger` exposes the RMT version slot; updates on every
--     deposit/withdraw/swap event.
--   • `created_at_ledger_derived` = first-ever snapshot of this pool.
--     One pool-pinned `MIN(ledger_sequence)` over snapshots is cheap.
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
--   • **task 0199:** `tvl` / `volume` / `fee_revenue` are NO LONGER read
--     from the snapshot columns (never populated — pre-0199 design). The
--     handler runs a second, small query and computes USD in Rust:
--       tvl         = latest reserves × last hourly USD close
--                     (`prices.price_usd_series_1h`, 48h lookback, per-leg
--                      identity, `close_usd > 0` only — non-positive rows
--                      are the prices-side 0171 sentinel, treated as
--                      absent; NULL unless BOTH legs price)
--       volume      = last-24h `sum(gross_volume_a)` (pool-pinned seek,
--                     `LIMIT 1 BY ledger_sequence` dedup) × leg-A close
--       fee_revenue = volume × fee_bps / 10000
--     NOT `prices.current_price_usd`: box-measured 2026-08-04 the spot view
--     carries the 0-sentinel for native XLM itself (their task 0135), so
--     every XLM-leg pool would read NULL; the 1h close is ≤ ~2h stale, same
--     cost (112 ms / 1.6M rows on the hottest pool), and matches the
--     chart's last bucket. Prices errors DEGRADE to NULL fields
--     (error-logged), never a 500. Deploy gate: API CH user needs SELECT
--     on `prices.*`.

SELECT
    lower(hex(lp.pool_id))                                                          AS pool_id_hex,
    toInt16(lp.pool_kind)                                                           AS pool_kind,
    lp.legs                                                                         AS legs,
    lp.fee_bps,
    toDecimal64(lp.fee_bps, 2) / 100                                                AS fee_percent,
    lp.last_updated_ledger,
    (SELECT min(ledger_sequence) FROM liquidity_pool_snapshots FINAL WHERE pool_id = $1)  AS created_at_ledger_derived,
    s.latest_ledger_sequence                                                        AS latest_snapshot_ledger,
    s.reserve_a,
    s.reserve_b,
    s.total_shares,
    -- s.tvl / s.volume / s.fee_revenue: not read (task 0199 — see Notes;
    -- USD analytics come from the separate compute-at-read query)
    l_snap.closed_at                                                                AS latest_snapshot_at
FROM liquidity_pools lp FINAL
LEFT JOIN (
    SELECT
        max(ledger_sequence)                      AS latest_ledger_sequence,
        argMax(reserve_a,        ledger_sequence) AS reserve_a,
        argMax(reserve_b,        ledger_sequence) AS reserve_b,
        argMax(total_shares,     ledger_sequence) AS total_shares
    FROM liquidity_pool_snapshots FINAL
    WHERE pool_id = $1
) s ON 1=1
LEFT JOIN ledgers l_snap ON l_snap.sequence = s.latest_ledger_sequence
WHERE lp.pool_id = $1;
