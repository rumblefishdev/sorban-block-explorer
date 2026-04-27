-- Task 0161: seed the native XLM singleton in `assets`.
--
-- The schema enforces native XLM as a singleton via
--   uidx_assets_native ON assets ((asset_type)) WHERE asset_type = 0
-- and ck_assets_identity requires asset_code/issuer_id/contract_id all NULL
-- for asset_type = 0. No upstream code path emits this row — neither the
-- parser (no native branch in detect_assets) nor migration 0005 (table DDL
-- only). This forward-only seed installs it once.
--
-- Plain INSERT (no ON CONFLICT): sqlx tracks `_sqlx_migrations` and runs
-- this exactly once per DB. If the row somehow exists already (manual
-- operator action), the INSERT fails loudly — that is the desired
-- behaviour over silent suppression.

INSERT INTO assets (asset_type, name)
VALUES (0, 'Stellar Lumen');
