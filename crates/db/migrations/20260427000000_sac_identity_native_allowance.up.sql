-- Task 0160: permit native XLM-SAC rows with NULL asset_code/issuer_id.
-- Stellar has no canonical issuer for native XLM (Horizon/SDK render it as
-- `{"asset_type":"native"}` with no issuer); representing the XLM-SAC wrap
-- with a synthesised sentinel account diverges from spec and leaks into
-- downstream API/UX. Loosen ck_assets_identity so native XLM-SAC stores as
-- (asset_type=2, asset_code=NULL, issuer_id=NULL, contract_id=<CSAC>).
--
-- Partial unique uidx_assets_soroban (contract_id WHERE asset_type IN (2,3))
-- still guarantees one row per SAC contract; uidx_assets_classic_asset
-- (code, issuer WHERE asset_type IN (1,2)) continues to dedupe classic SACs
-- and is not affected by NULL+NULL rows (treated as distinct by PG, but
-- dedupe for the native case is handled by uidx_assets_soroban).

ALTER TABLE assets DROP CONSTRAINT ck_assets_identity;

ALTER TABLE assets ADD CONSTRAINT ck_assets_identity CHECK (
    (asset_type = 0  -- native (classic)
        AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
 OR (asset_type = 1  -- classic_credit
        AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
 OR (asset_type = 2  -- sac (classic-wrap: code+issuer+contract; native-wrap: contract only)
        AND contract_id IS NOT NULL
        AND (
            (asset_code IS NOT NULL AND issuer_id IS NOT NULL)  -- classic SAC
         OR (asset_code IS NULL     AND issuer_id IS NULL)       -- native XLM-SAC
        ))
 OR (asset_type = 3  -- soroban
        AND issuer_id IS NULL      AND contract_id IS NOT NULL)
);
