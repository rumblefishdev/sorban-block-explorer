-- Endpoint:     GET /assets
-- Purpose:      Paginated list of assets across all four types (native,
--               classic credit, SAC, soroban-native). Optional filters:
--               type, asset_code (substring search via trigram).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.8
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :limit         INT       page size
--   $2  :cursor_id     INT       NULL on first page
--   $3  :asset_type    SMALLINT  NULL = no filter (token_asset_type domain
--                                0=native | 1=classic_credit | 2=sac | 3=soroban)
--   $4  :asset_code    VARCHAR   NULL = no filter; non-NULL is treated as a
--                                substring pattern via trigram (`% ... %`)
-- Indexes:      assets PK (id),
--               idx_assets_type           ON (asset_type),
--               idx_assets_code_trgm      GIN ON (asset_code gin_trgm_ops).
-- Notes:
--   • Keyset on `id DESC` — assets has a SERIAL surrogate so id ordering is
--     stable and roughly time-correlated. We do NOT order by holder_count
--     or total_supply (no covering index, would force a full sort).
--   • The asset_code filter uses ILIKE with leading `%` — that pattern is
--     served by `idx_assets_code_trgm` (gin_trgm_ops). Without the trigram
--     GIN, leading `%` would defeat any btree index. The API caller must
--     not pass `%` literals; the SQL wraps the parameter in `%...%` itself.
--   • `asset_type` is decoded via token_asset_type_name() in projection
--     only; WHERE compares to the SMALLINT literal so idx_assets_type stays
--     usable. (Note: the helper here is `token_asset_type_name`, not
--     `asset_type_name` — see ADR 0037 §"Enum label helper functions".)
--   • Issuer StrKey via final join to accounts; NULL for native and soroban
--     assets per the assets-identity check.
--   • Contract StrKey via final join to soroban_contracts; NULL for native
--     and classic_credit per the same identity check.

SELECT
    a.id,
    token_asset_type_name(a.asset_type)  AS asset_type_name,
    a.asset_type                         AS asset_type,
    a.asset_code,
    iss.account_id                       AS issuer,
    sc.contract_id                       AS contract_id,
    a.name,
    a.total_supply,
    a.holder_count,                      -- may be NULL or stale: ongoing tracking
                                         -- is blocked behind task 0135
                                         -- (token-holder-count-tracking).
    a.icon_url
FROM assets a
LEFT JOIN accounts          iss ON iss.id = a.issuer_id
LEFT JOIN soroban_contracts sc  ON sc.id  = a.contract_id
WHERE
    ($2::int      IS NULL OR a.id < $2)
    AND ($3::smallint IS NULL OR a.asset_type = $3)
    AND ($4::text     IS NULL OR a.asset_code ILIKE '%' || $4 || '%')
ORDER BY a.id DESC
LIMIT $1;
