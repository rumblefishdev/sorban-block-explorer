-- ============================================================================
-- liquidity_pools — unpartitioned. Classic LP registry.
-- Columns: pool_id, asset_a_type, asset_a_code, asset_a_issuer_id,
--          asset_b_type, asset_b_code, asset_b_issuer_id, fee_bps, created_at_ledger
-- ============================================================================
\echo '## liquidity_pools'

\echo '### I1 — pool_id is 32 bytes (SHA-256 of asset pair per Stellar protocol)'
SELECT COUNT(*) AS violations
FROM liquidity_pools WHERE octet_length(pool_id) <> 32;

\echo '### I2 — pool_id UNIQUE (PK)'
SELECT COUNT(*) AS violations
FROM (SELECT pool_id FROM liquidity_pools GROUP BY pool_id HAVING COUNT(*) > 1) d;

\echo '### I3 — asset_a < asset_b ordering enforced (Stellar canonicalises pair order)'
-- Two assets in a pool are ordered: native(0) < classic(1). Within same type, code asc, then issuer asc.
SELECT COUNT(*) AS violations,
       (SELECT array_agg(encode(pool_id,'hex')) FROM (
           SELECT pool_id FROM liquidity_pools
           WHERE asset_a_type > asset_b_type
              OR (asset_a_type = asset_b_type AND asset_a_code > asset_b_code)
              OR (asset_a_type = asset_b_type AND asset_a_code = asset_b_code
                  AND asset_a_issuer_id > asset_b_issuer_id)
           ORDER BY pool_id LIMIT 5
       ) s) AS sample
FROM liquidity_pools
WHERE asset_a_type > asset_b_type
   OR (asset_a_type = asset_b_type AND asset_a_code > asset_b_code)
   OR (asset_a_type = asset_b_type AND asset_a_code = asset_b_code
       AND asset_a_issuer_id > asset_b_issuer_id);

\echo '### I4 — issuer FK valid where set (asset_a, asset_b)'
SELECT
    (SELECT COUNT(*) FROM liquidity_pools lp
     LEFT JOIN accounts a ON a.id = lp.asset_a_issuer_id
     WHERE lp.asset_a_issuer_id IS NOT NULL AND a.id IS NULL) AS asset_a_violations,
    (SELECT COUNT(*) FROM liquidity_pools lp
     LEFT JOIN accounts a ON a.id = lp.asset_b_issuer_id
     WHERE lp.asset_b_issuer_id IS NOT NULL AND a.id IS NULL) AS asset_b_violations;

\echo '### I5 — fee_bps in [0, 10000] (basis points)'
SELECT COUNT(*) AS violations
FROM liquidity_pools WHERE fee_bps < 0 OR fee_bps > 10000;
