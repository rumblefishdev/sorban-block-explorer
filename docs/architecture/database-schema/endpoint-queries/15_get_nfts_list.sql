-- Endpoint:     GET /nfts
-- Purpose:      Paginated list of NFTs. Optional filters: collection name,
--               contract id, name substring (trigram).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.11
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :limit             INT       page size
--   $2  :cursor_id         INT       NULL on first page
--   $3  :collection_name   VARCHAR   NULL = no filter (exact match — there's
--                                    no trigram on collection_name; the index
--                                    is the plain btree idx_nfts_collection)
--   $4  :contract_strkey   VARCHAR   NULL = no filter; resolved to BIGINT id
--                                    via the UNIQUE on soroban_contracts
--   $5  :name              VARCHAR   NULL = no filter; substring match via
--                                    idx_nfts_name_trgm (gin_trgm_ops)
-- Indexes:      idx_nfts_collection ON (collection_name),
--               idx_nfts_owner      ON (current_owner_id),
--               idx_nfts_name_trgm  GIN ON (name gin_trgm_ops),
--               UNIQUE (contract_id, token_id),
--               soroban_contracts UNIQUE (contract_id) — for $4 resolve.
-- Notes:
--   • Keyset on `id DESC` (SERIAL surrogate, mint-time correlated). The
--     planner can use idx_nfts_collection / idx_nfts_name_trgm for filter
--     selectivity, then sort by id; with a small page size and high
--     filter selectivity this is much cheaper than ordering by mint time.
--   • The contract resolve uses a CTE so it runs once even when the
--     planner materializes idx_nfts_collection / idx_nfts_name_trgm.
--   • `collection_name` filter is exact (`=`) because the index is btree;
--     change to ILIKE only after adding a trigram index in task 0132.
--   • `name` filter wraps in `%...%` to leverage the trigram GIN; leading
--     `%` is required for trigram coverage.

WITH ct AS (
    SELECT id
    FROM soroban_contracts
    WHERE $4::varchar IS NOT NULL
      AND contract_id = $4
)
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
JOIN      soroban_contracts sc  ON sc.id = n.contract_id
LEFT JOIN accounts          own ON own.id = n.current_owner_id
WHERE
    ($2::int     IS NULL OR n.id < $2)
    AND ($3::varchar IS NULL OR n.collection_name = $3)
    AND ($4::varchar IS NULL OR n.contract_id = (SELECT id FROM ct))
    AND ($5::text    IS NULL OR n.name ILIKE '%' || $5 || '%')
ORDER BY n.id DESC
LIMIT $1;
