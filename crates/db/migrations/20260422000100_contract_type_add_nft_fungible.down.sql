-- Reverse of 20260422000100_contract_type_add_nft_fungible.up.sql.
-- Restores the two-variant `contract_type_name` body (token/other only)
-- that shipped with 20260422000000_enum_label_functions.up.sql.

CREATE OR REPLACE FUNCTION contract_type_name(ty SMALLINT) RETURNS TEXT
    IMMUTABLE PARALLEL SAFE LANGUAGE SQL AS $$
    SELECT CASE ty
        WHEN 0 THEN 'token'
        WHEN 1 THEN 'other'
    END
$$;
