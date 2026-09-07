-- Endpoint:     GET /nfts/:id
-- Purpose:      NFT detail: identity + media + current owner. The
--               `metadata` JSON blob is NOT served from this query — see
--               Notes (CH `nfts` has no metadata col per ADR 0044 §5.3,
--               same as PG post-migration-20260507120000).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.12
-- Schema:       ADR 0044 + PR-#175 hybrid-surrogate amendment + ADR 0043.
-- Data sources: DB columns + API-layer runtime fold-in for metadata
--               (Soroban RPC `token_uri()` + IPFS gateway).
-- Inputs:
--   $1  :contract_strkey  String  C-form contract ID
--   $2  :token_id         String  on-chain token_id (i128/u64/sym/Bytes as String)
-- Indexes:      nfts ORDER BY (contract_id, token_id) — direct PK seek
--                 after FINAL (PR #175 dropped surrogate `id`).
--               soroban_contracts ORDER BY (contract_id) — StrKey resolve.
--               accounts ORDER BY (account_id) — owner LEFT JOIN by id.
-- CH Engine:    nfts — Replacing(current_owner_ledger) (FINAL).
--               soroban_contracts, accounts — Replacing (FINAL).
-- CH Pattern:   FINAL'd direct seek on natural composite key. Inputs
--               changed from single `:id` Int32 (PG surrogate) to
--               (contract_strkey, token_id) tuple per PR #175.
-- ADR 0044 §:   §4.6 (nfts Replacing, no metadata col), §5.3 (divergence).
--   **PR #175 amendment:** inputs changed from `$1 = id Int32` to
--   `(contract_strkey, token_id)` tuple — surrogate dropped.
-- Notes:
--   • PG E16 keeps single `:id` Int32. CH has no surrogate; API passes
--     the tuple. Migration paths: (a) PG-backed id→tuple lookup; (b)
--     frontend uses synthetic cityHash64 surrogate from E15 as routing
--     key and the API decomposes.
--   • Burned NFT: `current_owner_id IS NULL` → LEFT JOIN yields NULL. Note
--     the burn ALSO erases `nfts.minted_at_ledger` — see the next bullet.
--   • **`minted_at_ledger` is DERIVED from `nft_ownership`, not read from the
--     `nfts` column (task 0528).** `nfts` is Replacing(current_owner_ledger)
--     with one row per token, so the burn above — or any later transfer —
--     replaces the WHOLE row with one carrying no mint ledger. Detail scopes
--     the derivation to the resolved contract:
--         LEFT JOIN (SELECT contract_id, token_id,
--                           min(ledger_sequence) AS minted_at_ledger
--                      FROM nft_ownership
--                     WHERE contract_id IN (SELECT id FROM cid)
--                       AND event_type = 0
--                     GROUP BY contract_id, token_id) mi
--     `event_type = 0` and the `nullIf(_, 0)` wrapper are both load-bearing —
--     see `15_get_nfts_list.sql` for the full reasoning. `nfts.minted_at_ledger`
--     stays written and unread until task 0529 drops it.
--
-- ---------------------------------------------------------------------------
-- DRIFT NOTICE — as with E15, the statement below is an intent sketch, not the
-- query the API runs. `crates/api/src/nfts/queries.rs::fetch_by_composite` is
-- authoritative: it serves collection / name / media from the `nft_enrichment`
-- collapse (+ `soroban_contract_metadata` ledger-name precedence), resolves the
-- owner StrKey in Rust via a `WHERE id IN` seek instead of an `accounts` FINAL
-- join, and echoes the contract StrKey from the request input (task 0355).
-- ---------------------------------------------------------------------------

SELECT
    n.contract_id                             AS contract_id_raw,
    sc.contract_id                            AS contract_id_strkey,
    n.token_id,
    n.collection_name,
    n.name,
    n.media_url,
    nullIf(mi.minted_at_ledger, 0)            AS minted_at_ledger,
    own.account_id                            AS current_owner,
    n.current_owner_ledger
FROM nfts n FINAL
JOIN      soroban_contracts sc  FINAL ON sc.id  = n.contract_id
LEFT JOIN accounts          own FINAL ON own.id = n.current_owner_id AND n.current_owner_id IS NOT NULL
LEFT JOIN (
    SELECT contract_id, token_id, min(ledger_sequence) AS minted_at_ledger
      FROM nft_ownership
     WHERE event_type = 0
     GROUP BY contract_id, token_id
) mi ON mi.contract_id = n.contract_id AND mi.token_id = n.token_id
WHERE n.contract_id = (SELECT id FROM soroban_contracts FINAL WHERE contract_id = $1 LIMIT 1)
  AND n.token_id    = $2;
