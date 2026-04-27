-- Revert task 0161 seed. Destructive — removes the native XLM singleton
-- and any operator-added native rows alongside it. Any code path that
-- reads `assets.asset_type = 0` (frontend XLM detail, /assets listing)
-- will see an empty result after this runs.

DELETE FROM assets WHERE asset_type = 0;
