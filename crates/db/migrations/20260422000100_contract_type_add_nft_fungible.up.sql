-- Task 0118 Phase 2: extend the `contract_type_name(SMALLINT)` label helper
-- to cover the two new `domain::ContractType` variants (`Nft = 2`,
-- `Fungible = 3`). The `ck_sc_contract_type_range` CHECK already permits
-- 0..15 so no column change is needed — this migration only teaches the
-- SQL-side pretty-printer the new discriminants.

CREATE OR REPLACE FUNCTION contract_type_name(ty SMALLINT) RETURNS TEXT
    IMMUTABLE PARALLEL SAFE LANGUAGE SQL AS $$
    SELECT CASE ty
        WHEN 0 THEN 'token'
        WHEN 1 THEN 'other'
        WHEN 2 THEN 'nft'
        WHEN 3 THEN 'fungible'
    END
$$;
