-- ============================================================================
-- assets — unified registry (native, classic_credit, SAC, Soroban-native).
-- Columns: id, asset_type, asset_code, issuer_id, contract_id, name,
--          total_supply, holder_count, icon_url
-- Constraints: ck_assets_identity (per ADR 0023 + ADR 0038), uidx_assets_native,
-- uidx_assets_classic_asset, uidx_assets_soroban
-- ============================================================================
\echo '## assets'

\echo '### I1 — asset_type SMALLINT in known range (0-3 per ADR 0036)'
SELECT COUNT(*) AS violations
FROM assets WHERE asset_type < 0 OR asset_type > 3;

\echo '### I2 — ck_assets_identity per ADR 0038 (validate shape per type)'
-- type=0 native: code=NULL, issuer=NULL, contract=NULL
-- type=1 classic_credit: code+issuer NOT NULL, contract=NULL
-- type=2 SAC: contract NOT NULL, code+issuer either both set (classic SAC) or both NULL (native SAC)
-- type=3 Soroban-native: issuer=NULL, contract NOT NULL
SELECT COUNT(*) AS violations,
       (SELECT array_agg(id) FROM (
           SELECT id FROM assets WHERE NOT (
               (asset_type = 0 AND asset_code IS NULL AND issuer_id IS NULL AND contract_id IS NULL)
            OR (asset_type = 1 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
            OR (asset_type = 2 AND contract_id IS NOT NULL AND
                ((asset_code IS NOT NULL AND issuer_id IS NOT NULL)
                 OR (asset_code IS NULL AND issuer_id IS NULL)))
            OR (asset_type = 3 AND issuer_id IS NULL AND contract_id IS NOT NULL)
           ) ORDER BY id LIMIT 5
       ) s) AS sample
FROM assets
WHERE NOT (
    (asset_type = 0 AND asset_code IS NULL AND issuer_id IS NULL AND contract_id IS NULL)
 OR (asset_type = 1 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
 OR (asset_type = 2 AND contract_id IS NOT NULL AND
     ((asset_code IS NOT NULL AND issuer_id IS NOT NULL)
      OR (asset_code IS NULL AND issuer_id IS NULL)))
 OR (asset_type = 3 AND issuer_id IS NULL AND contract_id IS NOT NULL)
);

\echo '### I3 — uidx_assets_native singleton (exactly one row with asset_type=0)'
SELECT
    CASE WHEN COUNT(*) <> 1 THEN 1 ELSE 0 END AS violations,
    COUNT(*) AS native_row_count
FROM assets WHERE asset_type = 0;

\echo '### I4 — issuer_id FK valid where set'
SELECT COUNT(*) AS violations
FROM assets a
LEFT JOIN accounts acc ON acc.id = a.issuer_id
WHERE a.issuer_id IS NOT NULL AND acc.id IS NULL;

\echo '### I5 — contract_id FK to soroban_contracts valid where set'
SELECT COUNT(*) AS violations
FROM assets a
LEFT JOIN soroban_contracts c ON c.id = a.contract_id
WHERE a.contract_id IS NOT NULL AND c.id IS NULL;

\echo '### I6 — non-negative supply / holder count'
SELECT COUNT(*) AS violations
FROM assets WHERE holder_count < 0 OR (total_supply IS NOT NULL AND total_supply < 0);
