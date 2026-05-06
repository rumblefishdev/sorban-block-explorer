-- Endpoint:     GET /assets/:id
-- Purpose:      Asset detail. DB returns the typed-metadata header (code,
--               type, supply, holder_count, icon, name) plus the issuer's
--               on-chain home_domain used as the SEP-1 lookup key. The API
--               then runs a runtime SEP-1 fetch against the issuer's
--               stellar.toml to overlay description + home_page.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.9
-- Schema:       ADR 0037
-- Data sources: DB + runtime SEP-1 HTTP fetch.
--               DB returns: id, asset_type, asset_code, issuer, contract_id,
--                           name, total_supply, holder_count, icon_url,
--                           deployed_at_ledger (Soroban only),
--                           issuer_home_domain (internal SEP-1 lookup key,
--                           not in API response).
--               Runtime returns: description, home_page
--                           — fetched per request from
--                           `https://{issuer_home_domain}/.well-known/stellar.toml`
--                           via `runtime_enrichment::sep1` (task 0188).
--                           `description` ← `CURRENCIES[].desc`;
--                           `home_page`   ← `DOCUMENTATION.ORG_URL`.
--                           Replaces the abandoned per-entity S3 blob plan
--                           (task 0164 / ADR 0037 §11). Failure / no
--                           home_domain → both fields NULL.
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
--   • `issuer_home_domain` is projected as an internal lookup key — the API
--     consumes it to decide whether to issue the SEP-1 fetch and what host
--     to hit, but the column itself is NOT serialised onto the response.

SELECT
    a.id,
    token_asset_type_name(a.asset_type) AS asset_type_name,
    a.asset_type                        AS asset_type,
    a.asset_code,
    iss.account_id                      AS issuer,
    iss.home_domain                     AS issuer_home_domain, -- internal SEP-1 lookup key
    sc.contract_id                      AS contract_id,
    a.name,
    a.total_supply,
    a.holder_count,                     -- may be NULL or stale: ongoing tracking
                                        -- is blocked behind task 0135
                                        -- (token-holder-count-tracking).
    a.icon_url,
    sc.deployed_at_ledger               AS deployed_at_ledger
    -- not in DB: description, home_page — runtime SEP-1 fetch via
    --   `runtime_enrichment::sep1` (task 0188), keyed off issuer_home_domain.
FROM assets a
LEFT JOIN accounts          iss ON iss.id = a.issuer_id
LEFT JOIN soroban_contracts sc  ON sc.id  = a.contract_id
WHERE a.id = $1;
