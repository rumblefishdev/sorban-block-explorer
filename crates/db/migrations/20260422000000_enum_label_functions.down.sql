-- Reverse of 20260422000000_enum_label_functions.up.sql (ADR 0031).
-- Drops the six IMMUTABLE SQL helpers that render readable labels for
-- each SMALLINT enum column. The SMALLINT columns themselves live in
-- the 0002-0007 baseline and are irreversible per MIGRATIONS.md §3.

DROP FUNCTION IF EXISTS contract_type_name(SMALLINT);
DROP FUNCTION IF EXISTS nft_event_type_name(SMALLINT);
DROP FUNCTION IF EXISTS event_type_name(SMALLINT);
DROP FUNCTION IF EXISTS token_asset_type_name(SMALLINT);
DROP FUNCTION IF EXISTS asset_type_name(SMALLINT);
DROP FUNCTION IF EXISTS op_type_name(SMALLINT);
