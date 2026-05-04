-- Endpoint:     GET /liquidity-pools/:id
-- Purpose:      Pool detail: identity (asset pair, fee, created_at_ledger) +
--               latest on-chain state (reserves, total shares, TVL).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.14
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :pool_id            BYTEA(32)  raw 32-byte pool id
-- Indexes:      liquidity_pools PK (pool_id),
--               idx_lps_pool ON (pool_id, created_at DESC).
-- Notes:
--   • Single statement. The latest-snapshot subquery is `LIMIT 1` on
--     `idx_lps_pool` — one index seek.
--   • No freshness-window predicate. Pool reserves/total_shares only
--     change on deposit/withdraw/swap events (snapshot triggers are
--     state-change driven — see `xdr_parser::extract_liquidity_pools`),
--     so the latest snapshot is always the actual current on-chain state
--     regardless of its age. Clients that care about freshness can read
--     `latest_snapshot_at` in the response. `tvl`/`volume`/`fee_revenue`
--     are NULL today (populated by a future TVL-ingestion task).
--   • Issuer StrKeys via final joins. Native legs (asset_*_type = 0) have
--     NULL issuer_id; LEFT JOIN yields NULL.

SELECT
    encode(lp.pool_id, 'hex')          AS pool_id_hex,
    asset_type_name(lp.asset_a_type)   AS asset_a_type_name,
    lp.asset_a_type                    AS asset_a_type,
    lp.asset_a_code,
    iss_a.account_id                   AS asset_a_issuer,
    asset_type_name(lp.asset_b_type)   AS asset_b_type_name,
    lp.asset_b_type                    AS asset_b_type,
    lp.asset_b_code,
    iss_b.account_id                   AS asset_b_issuer,
    lp.fee_bps,
    -- Frontend §6.14 shows "fee percentage". Conversion done here.
    (lp.fee_bps::numeric / 100)        AS fee_percent,
    lp.created_at_ledger,
    s.ledger_sequence                  AS latest_snapshot_ledger,
    s.reserve_a,
    s.reserve_b,
    s.total_shares,
    s.tvl,
    s.volume,
    s.fee_revenue,
    s.created_at                       AS latest_snapshot_at
FROM liquidity_pools lp
LEFT JOIN accounts iss_a ON iss_a.id = lp.asset_a_issuer_id
LEFT JOIN accounts iss_b ON iss_b.id = lp.asset_b_issuer_id
LEFT JOIN LATERAL (
    SELECT
        lps.ledger_sequence,
        lps.reserve_a,
        lps.reserve_b,
        lps.total_shares,
        lps.tvl,
        lps.volume,
        lps.fee_revenue,
        lps.created_at
    FROM liquidity_pool_snapshots lps
    WHERE lps.pool_id = lp.pool_id
    ORDER BY lps.created_at DESC, lps.ledger_sequence DESC
    LIMIT 1
) s ON TRUE
WHERE lp.pool_id = $1;
