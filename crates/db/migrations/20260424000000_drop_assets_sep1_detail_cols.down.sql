-- Reverse of 20260424000000_drop_assets_sep1_detail_cols.up.sql.
-- Re-adds `description` and `home_page` as nullable columns (shape
-- from ADR 0023 Part 3). Data that was in these columns before the
-- drop is not restored — per the 2026-04-10 pipeline audit there was
-- no such data (always NULL), so no backfill is required.

ALTER TABLE assets ADD COLUMN description TEXT;
ALTER TABLE assets ADD COLUMN home_page   VARCHAR(256);
