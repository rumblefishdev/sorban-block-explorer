-- Endpoint:     GET /nfts/:id
-- Purpose:      NFT detail: identity + media + metadata + current owner.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.12
-- Schema:       ADR 0037
-- Data sources: DB-only (today). The `metadata` JSONB column carries the
--               full attribute set; if a per-NFT S3 enrichment layout
--               lands later (parallel to assets/{id}.json), revise to
--               overlay it. Until then JSONB is the full source.
-- Inputs:
--   $1  :id  INT  NFT surrogate id
-- Indexes:      nfts PK (id),
--               soroban_contracts PK (id) for contract join,
--               accounts PK (id) for owner join.
-- Notes:
--   • Single statement. Owner LEFT JOIN tolerates NULL — happens for
--     burned NFTs (current_owner_id NULL) per ADR 0037 §13.
--   • SCHEMA-DOC GAP: the JSONB shape of `nfts.metadata` is contract-
--     defined at mint time and NOT standardized in `docs/architecture/**`.
--     Frontend §6.12 needs to render the "full attribute list (traits,
--     properties)" — that requires either a documented canonical shape
--     (so the frontend knows where to look) or a defensive UI that walks
--     arbitrary JSONB. Until a canonical shape is locked, the API returns
--     the JSONB verbatim. If a per-NFT S3 enrichment layout lands later
--     (parallel to assets/{id}.json), revise this file to overlay it.

SELECT
    n.id,
    sc.contract_id,
    n.token_id,
    n.collection_name,
    n.name,
    n.media_url,
    n.metadata,
    n.minted_at_ledger,
    own.account_id    AS current_owner,
    n.current_owner_ledger
FROM nfts n
JOIN      soroban_contracts sc  ON sc.id  = n.contract_id
LEFT JOIN accounts          own ON own.id = n.current_owner_id
WHERE n.id = $1;
