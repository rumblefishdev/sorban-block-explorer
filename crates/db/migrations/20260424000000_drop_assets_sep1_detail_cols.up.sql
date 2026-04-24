-- Task 0164: drop `assets.description` and `assets.home_page`.
-- Asset-detail SEP-1 metadata now served from S3 (`assets/{id}.json`) as
-- recorded in ADR 0037 (post-drop schema snapshot; no separate
-- superseding ADR). `icon_url` intentionally retained for list-level
-- thumbnail rendering (it is NOT detail-only).
--
-- Zero data loss: the 2026-04-10 pipeline audit confirmed these columns
-- were always NULL (no enrichment path ever wrote them).

ALTER TABLE assets DROP COLUMN description;
ALTER TABLE assets DROP COLUMN home_page;
