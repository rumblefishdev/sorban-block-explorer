-- Endpoint:     GET /assets/:id
-- Purpose:      Asset detail. DB returns the typed-metadata header (code,
--               type, supply, holder_count, icon, name); description and
--               home_page are overlaid by the API from a per-entity S3 blob.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.9
-- Schema:       ADR 0037
-- Data sources: DB + S3 per-entity blob.
--               DB returns: id, asset_type, asset_code, issuer, contract_id,
--                           name, total_supply, holder_count, icon_url,
--                           deployed_at_ledger (Soroban only).
--               S3 returns: description, home_page
--                           — `s3://<bucket>/assets/{id}.json`
--                           per ADR 0037 §11 / task 0164 (off-chain SEP-1
--                           enrichment, not derived from XDR).
-- Inputs:
--   $1  :id  INT  asset surrogate id (the SERIAL PK; the API resolves
--                  StrKey/contract identity to this id at the request boundary)
-- Indexes:      assets PK (id),
--               accounts PK (id) for issuer join,
--               soroban_contracts PK (id) for contract join.
-- Notes:
--   • Single statement. All identity branching lives in the assets row
--     itself (asset_type CHECK constraint guarantees the issuer/contract
--     columns are populated correctly per type), so the LEFT JOINs are
--     safe and just yield NULL for the irrelevant columns.
--   • `deployed_at_ledger` is sourced from `soroban_contracts.deployed_at_ledger`
--     for SAC and Soroban-native types; classic and native return NULL.

SELECT
    a.id,
    token_asset_type_name(a.asset_type) AS asset_type_name,
    a.asset_type                        AS asset_type,
    a.asset_code,
    iss.account_id                      AS issuer,
    sc.contract_id                      AS contract_id,
    a.name,
    a.total_supply,
    a.holder_count,                     -- may be NULL or stale: ongoing tracking
                                        -- is blocked behind task 0135
                                        -- (token-holder-count-tracking).
    a.icon_url,
    sc.deployed_at_ledger               AS deployed_at_ledger
    -- not in DB: description, home_page — S3 assets/{id}.json (ADR 0037 §11, task 0164).
FROM assets a
LEFT JOIN accounts          iss ON iss.id = a.issuer_id
LEFT JOIN soroban_contracts sc  ON sc.id  = a.contract_id
WHERE a.id = $1;
