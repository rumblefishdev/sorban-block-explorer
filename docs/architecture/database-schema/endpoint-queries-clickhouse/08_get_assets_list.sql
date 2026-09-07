-- ⚠️ SUPERSEDED by task 0331 (unified balances). `total_supply` / `holder_count` now
-- come from `balance_aggregates` (refreshable MV over the unified `balances` table,
-- keyed by the RE-ADDED `assets.id` surrogate — PR #175 dropped it, 0331 restored it),
-- NOT `asset_aggregates`. Post-ADR-0051 a SAC is a FACET folded into its classic row,
-- so there is ONE aggregate row per asset (no separate SAC row). Authoritative query:
-- `crates/api/src/assets/queries_ch.rs`. Reference SQL below is pre-0331.
-- Endpoint:     GET /assets
-- Purpose:      Paginated list of assets across all four types (native,
--               classic credit, SAC, soroban-native). Optional filters:
--               type, asset_code (substring search).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.8
-- Schema:       ADR 0044 + PR-#175 hybrid-surrogate amendment.
-- Data sources: DB-only.
-- Inputs:
--   $1  :limit                 Int     page size
--   $2  :cursor_asset_type     Int16   NULL on first page (tuple cursor)
--   $3  :cursor_asset_code     String  NULL on first page
--   $4  :cursor_issuer_id      Int64   NULL on first page
--   $5  :cursor_contract_id    Int64   NULL on first page
--   $6  :asset_type_filter     Int16   NULL = no filter
--   $7  :asset_code_filter     String  NULL = no filter (substring)
-- Indexes:      assets ORDER BY (asset_type, asset_code, issuer_id, contract_id)
--                 — natural composite key (PR #175 dropped surrogate `id`).
--               soroban_contracts ORDER BY (contract_id) — contract join via id.
--               soroban_contract_metadata ORDER BY (contract_id) — metadata join.
--               asset_enrichment / asset_aggregates — side-table joins (below).
-- CH Engine:    assets — ReplacingMergeTree (no version) (FINAL required).
--               soroban_contracts — Replacing (joined WITHOUT FINAL; a join
--                 miss is neutralised by `nullIf`, the ~18M-row FINAL avoided).
--               soroban_contract_metadata — Replacing(version) (FINAL in the
--                 sub-select → latest row per contract_id).
--               asset_enrichment — Replacing(version), collapsed via argMax in
--                 a GROUP BY sub-select (no FINAL).
--               asset_aggregates — MergeTree batch table (no FINAL).
-- CH Pattern:   FINAL on `assets` + the metadata sub-select only. Keyset cursor
--               on the 4-tuple natural key, ASC (task 0485).
--               asset_code substring against the DISPLAYED code (no pg_trgm in
--               CH; linear scan acceptable on small `assets`).
-- ADR 0044 §:   §4.5 (Replacing state). **PR #175 amendment:** `assets`
--               dropped surrogate `id Int32`; natural composite key now.
--               `issuer_id` / `contract_id` are `Int64` (NOT Nullable);
--               `0` is the sentinel for "no issuer" / "no contract" (native
--               XLM: both 0; classic credit: contract_id=0; soroban-native:
--               issuer_id=0).
-- Notes:
--   • PR #175 dropped `assets.id`. Route via the 4-tuple natural key (or a
--     synthesized `cityHash64(...)` at the API layer).
--   • **List variant (task 0319): NO `accounts` issuer join.** The issuer
--     StrKey + home_domain are resolved per page by a bloom-pruned key-seek;
--     the ~18M-row `accounts` hash-build was the dominant list cost. The
--     detail paths (09) keep the issuer join. `issuer_id_key` is exposed for
--     the cursor + that per-page seek.
--   • **name / symbol / decimals are read-composed from side tables, NOT from
--     the `assets` row.** `assets.name` has had no writer since task 0297
--     (empty going forward) and no reader; pending `DROP COLUMN` (task 0310). Name
--     precedence: `asset_enrichment.name` (classic/SAC enrichment, task 0231)
--     → `soroban_contract_metadata.name` (on-chain SEP-41 `METADATA`, task
--     0297) → `'Stellar Lumens'` literal for native. `symbol` / `decimals`
--     come from `soroban_contract_metadata` (decimals defaults to 7 for
--     classic/SAC). PG still reads `a.name` until 0243 (see the PG snapshot).

SELECT
    a.asset_type                              AS asset_type,
    nullIf(a.asset_code, '')                  AS asset_code,
    nullIf(sc.contract_id, '')                AS contract_id,
    coalesce(nullIf(ae.name, ''), nullIf(m.name, ''),
             if(a.asset_type = 0, 'Stellar Lumens', NULL)) AS name,
    nullIf(m.symbol, '')                      AS symbol,         -- on-chain token symbol (0297); NULL for classic/native
    coalesce(m.decimals, 7)                   AS decimals,       -- on-chain decimals (0297); 7 = classic/SAC default
    toString(agg.total_supply)                AS total_supply,   -- asset_aggregates (task 0293)
    agg.holder_count                          AS holder_count,
    nullIf(sc.deployed_at_ledger, 0)          AS deployed_at_ledger,
    nullIf(ae.icon_url, '')                   AS icon_url,       -- asset_enrichment (task 0231)
    a.issuer_id                               AS issuer_id_key,  -- cursor + per-page issuer seek
    a.contract_id                             AS contract_id_key
FROM assets a FINAL
LEFT JOIN soroban_contracts sc ON sc.id = a.contract_id
LEFT JOIN (
    SELECT contract_id, name, symbol, decimals
    FROM soroban_contract_metadata FINAL      -- task 0297 side table; RMT(version) → latest per contract
) m ON m.contract_id = sc.contract_id
LEFT JOIN asset_aggregates agg
       ON agg.asset_code = a.asset_code AND agg.issuer_id = a.issuer_id
LEFT JOIN (
    SELECT asset_type, asset_code, issuer_id, contract_id,
           argMax(icon_url, version) AS icon_url,
           argMax(name, version)     AS name
    FROM asset_enrichment                     -- task 0231 side table
    GROUP BY asset_type, asset_code, issuer_id, contract_id
) ae ON ae.asset_type = a.asset_type AND ae.asset_code = a.asset_code
    AND ae.issuer_id = a.issuer_id   AND ae.contract_id = a.contract_id
WHERE
    ($6 IS NULL OR a.asset_type = $6)
    -- Matched against the DISPLAYED code: native XLM stores an EMPTY code and
    -- renders as `XLM`, so comparing the stored value returned thousands of
    -- impostor codes and missed the one asset everybody meant. The same
    -- expression appears in 22_get_search.sql and in the pools predicate.
    AND ($7 IS NULL OR position(lower(if(a.asset_type = 0, 'XLM', toString(a.asset_code))),
                                lower($7)) > 0
                    OR positionCaseInsensitive(coalesce(m.name, ''), $7) > 0
                    OR positionCaseInsensitive(coalesce(m.symbol, ''), $7) > 0)
    AND ($2 IS NULL OR (asset_type, asset_code, issuer_id, contract_id)
                     > ($2, $3, $4, $5))
-- ASC, filtered or not, because native XLM is the MINIMUM of that 4-tuple:
-- under DESC it sorted onto the LAST page, so `filter[code]=xlm` answered with
-- `zXLMr, zXLM, …` and the unfiltered list opened the asset list of a Stellar
-- explorer on codeless Soroban contracts without ever showing XLM.
ORDER BY asset_type ASC, asset_code ASC, issuer_id ASC, contract_id ASC
LIMIT $1;

-- NO relevance ranking on this surface, by decision (task 0485). A tier order
-- would have to be carried in the cursor — a keyset must resume in the order it
-- walked — which means a rank column, a second copy of the tier rule in Rust to
-- mint that cursor, and a page-walk test to catch the two drifting. That was
-- built, measured and taken back out: this is a browse list with a filter, and
-- the walk direction alone answers "native first". Relevance lives in
-- 22_get_search.sql, which has no pagination and so pays none of it.

-- STALE ABOVE (not touched by task 0485): the projection still shows the
-- single-statement shape with `asset_aggregates`. The read has been a two-phase
-- keys-then-hydrate seek since task 0364, and holder counts come from
-- `balance_aggregates` since task 0331. Only the predicate, keyset and ORDER BY
-- below were brought up to date here.
