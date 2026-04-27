-- Revert ck_assets_identity to the pre-0160 strict form — asset_type=2
-- requires NOT NULL code+issuer+contract. Any native XLM-SAC rows with
-- NULL code/issuer must be purged before down migration runs, else
-- constraint addition fails.

ALTER TABLE assets DROP CONSTRAINT ck_assets_identity;

ALTER TABLE assets ADD CONSTRAINT ck_assets_identity CHECK (
    (asset_type = 0
        AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
 OR (asset_type = 1
        AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
 OR (asset_type = 2
        AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NOT NULL)
 OR (asset_type = 3
        AND issuer_id IS NULL      AND contract_id IS NOT NULL)
);
