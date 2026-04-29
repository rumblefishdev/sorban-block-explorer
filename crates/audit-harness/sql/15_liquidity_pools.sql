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

\echo '### I3 — asset_a < asset_b type/code ordering enforced (Stellar canonicalises pair order)'
-- Stellar canonical order is: type asc (native(0) < alphanum4(1) < alphanum12(2)),
-- then code asc, then issuer ed25519-raw-byte asc. The first two levels are
-- expressible in SQL on our schema (asset_*_type SMALLINT, asset_*_code TEXT
-- with codes already left-padded by the parser to a fixed width). The third
-- level — issuer order — is *not* SQL-expressible against this schema:
--
--   • `asset_*_issuer_id` is a surrogate BIGINT FK to `accounts.id`, assigned
--     in insertion order; it has zero correlation with the issuer's ed25519
--     raw byte value that the protocol uses for canonical comparison.
--   • The natural key `accounts.account_id` is a base32-encoded G-strkey;
--     base32's alphabet (A-Z = 0-25, 2-7 = 26-31) is monotonic for the
--     encoded payload BUT lexicographic ASCII string comparison is NOT
--     monotonic for that alphabet (digits 2-7 sort BEFORE letters A-Z in
--     ASCII while encoding higher base32 values), so `account_id > account_id`
--     does not preserve raw-byte order either.
--
-- The protocol-canonical order at the issuer level is therefore enforced
-- only via the `pool_id` itself: per CAP-0038, `pool_id =
-- SHA-256(LiquidityPoolParameters XDR)` over the asset pair in canonical
-- order plus the fee. That hash check requires reconstructing the XDR
-- (asset type + code + 32-byte ed25519 issuer + fee), which is performed
-- by the audit-harness Phase 2c archive XDR re-parse — see `archive-diff
-- --table liquidity_pools`. A SQL-only re-derivation would require base32
-- decoding the issuer strkey to raw bytes; pgcrypto's `digest()` is
-- available, but the byte reconstruction is not worth the surface area
-- for an invariant that is already covered by the archive cross-check.
--
-- This invariant is therefore deliberately scoped to (type, code) and
-- defers issuer-level canonical-order verification to Phase 2c. Pre-fix
-- versions of this query asserted `asset_a_issuer_id > asset_b_issuer_id`
-- and produced false positives proportional to the number of same-(type,
-- code) different-issuer pools whose surrogate IDs landed in
-- reverse-of-canonical insertion order. See task 0179 for the diagnosis.
SELECT COUNT(*) AS violations,
       (SELECT array_agg(encode(pool_id,'hex')) FROM (
           SELECT pool_id FROM liquidity_pools
           WHERE asset_a_type > asset_b_type
              OR (asset_a_type = asset_b_type AND asset_a_code > asset_b_code)
           ORDER BY pool_id LIMIT 5
       ) s) AS sample
FROM liquidity_pools
WHERE asset_a_type > asset_b_type
   OR (asset_a_type = asset_b_type AND asset_a_code > asset_b_code);

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
