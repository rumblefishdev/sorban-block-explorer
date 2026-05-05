-- Endpoint:     GET /search?q=&type=transaction,contract,asset,account,nft,pool
-- Purpose:      Unified search across all entity types. The API classifies
--               the query (hash-shape, StrKey-shape, plain text) and may
--               restrict via the `type` parameter; this SQL accepts the
--               classification + (optional) type allowlist as inputs and
--               returns grouped, capped result sets per entity type.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.15
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :q                 TEXT       the raw user query (UTF-8)
--   $2  :hash_bytes        BYTEA(32)  parsed hash bytes if $1 looked like a
--                                     hex/base64 32-byte hash; NULL otherwise
--   $3  :strkey_prefix     TEXT       upper-cased StrKey or its prefix (G…/C…)
--                                     if $1 looked like a StrKey; NULL otherwise
--   $4  :per_group_limit   INT        cap per entity bucket. The API MUST
--                                     pick a default (recommended: 10) and
--                                     a hard ceiling (recommended: 50) so
--                                     callers cannot expand the union to
--                                     scan all six indexes deeply. Document
--                                     the chosen default in the OpenAPI
--                                     spec so frontend can paginate
--                                     consistently.
--   $5  :include_tx        BOOLEAN
--   $6  :include_contract  BOOLEAN
--   $7  :include_asset     BOOLEAN
--   $8  :include_account   BOOLEAN
--   $9  :include_nft       BOOLEAN
--   $10 :include_pool      BOOLEAN
-- Indexes:      transaction_hash_index PK (hash),
--               idx_contracts_prefix    (contract_id text_pattern_ops),
--               idx_contracts_search    GIN (search_vector),
--               idx_assets_code_trgm    GIN (asset_code gin_trgm_ops),
--               idx_accounts_prefix     (account_id text_pattern_ops),
--               idx_nfts_name_trgm      GIN (name gin_trgm_ops),
--               liquidity_pools PK (pool_id) — exact-hex match only.
-- Notes:
--   • Single statement, UNION ALL of six narrow CTEs. Each CTE is bounded
--     `LIMIT $4` so the union is small; the outer SELECT preserves the
--     entity_type column so the API can group by it.
--   • CTE selection matches the input shape:
--       — tx       — exact match on `transaction_hash_index.hash`. Only
--                    fires when $2 is non-NULL (hash-shaped query).
--       — contract — StrKey prefix via idx_contracts_prefix when $3 is
--                    non-NULL; otherwise full-text on metadata via
--                    idx_contracts_search.
--       — asset    — trigram on asset_code via idx_assets_code_trgm.
--       — account  — StrKey prefix on idx_accounts_prefix; only when $3
--                    is non-NULL.
--       — nft      — trigram on name via idx_nfts_name_trgm; on a fully
--                    typed contract id ($3 starts with 'C…') the API
--                    SHOULD redirect at the route level and skip search.
--       — pool     — exact-hex pool_id match (32-byte BYTEA via $2) —
--                    pool ids are 32 bytes like hashes.
--   • The `:include_*` flags let the endpoint's `?type=` allowlist limit
--     the result set without changing the SQL shape; CTEs whose flag is
--     FALSE return zero rows because of the leading `WHERE FALSE`-style
--     guard. This is cheaper than building the SQL conditionally because
--     the planner removes the entire branch.
--   • Each result row carries: entity_type, identifier (the canonical
--     human-shown id), label (a short context string), and a stable
--     surrogate id (or NULL where the entity has none) for the API to
--     build the link target.

WITH
tx_hits AS (
    SELECT
        'transaction'::text       AS entity_type,
        encode(thi.hash, 'hex')   AS identifier,
        'ledger ' || thi.ledger_sequence::text AS label,
        NULL::bigint              AS surrogate_id
    FROM transaction_hash_index thi
    WHERE $5  = TRUE
      AND $2 IS NOT NULL
      AND thi.hash = $2
    LIMIT $4
),
contract_hits AS (
    SELECT
        'contract'::text          AS entity_type,
        sc.contract_id            AS identifier,
        COALESCE(sc.name, '')              AS label,
        sc.id                     AS surrogate_id
    FROM soroban_contracts sc
    WHERE $6 = TRUE
      AND (
              ( $3 IS NOT NULL AND sc.contract_id LIKE $3 || '%' )
           OR ( $3 IS NULL     AND sc.search_vector @@ plainto_tsquery('simple', $1) )
          )
    LIMIT $4
),
asset_hits AS (
    -- NOTE: `identifier` here is the asset CODE (display text), which is NOT
    -- a unique key for classic assets (multiple issuers may share a code).
    -- The frontend MUST route `/assets/:id` using `surrogate_id`, not
    -- `identifier`. `identifier` is for display only.
    SELECT
        'asset'::text                       AS entity_type,
        COALESCE(a.asset_code, 'XLM')       AS identifier,
        token_asset_type_name(a.asset_type) AS label,
        a.id::bigint                        AS surrogate_id
    FROM assets a
    WHERE $7 = TRUE
      AND (
              -- Classic / SAC / Soroban with an asset_code: trigram substring match.
              (a.asset_code IS NOT NULL AND a.asset_code ILIKE '%' || $1 || '%')
              -- Native XLM: explicit text match. asset_code IS NULL on native rows
              -- per the assets-identity CHECK, so the trigram path can't catch it.
           OR (a.asset_type = 0 AND ($1 ILIKE 'xlm' OR $1 ILIKE 'native'))
          )
    LIMIT $4
),
account_hits AS (
    SELECT
        'account'::text         AS entity_type,
        a.account_id            AS identifier,
        COALESCE(a.home_domain, '') AS label,
        a.id                    AS surrogate_id
    FROM accounts a
    WHERE $8 = TRUE
      AND $3 IS NOT NULL
      AND a.account_id LIKE $3 || '%'
    LIMIT $4
),
nft_hits AS (
    -- NOTE: `identifier` here is the NFT NAME (display text), which is NOT
    -- a unique key — multiple NFTs across collections can share a name. The
    -- frontend MUST route `/nfts/:id` using `surrogate_id`, not `identifier`.
    -- `identifier` is for display only. The natural key is
    -- `(contract_id, token_id)`; we don't surface it here because the
    -- surrogate is the route param.
    SELECT
        'nft'::text                          AS entity_type,
        n.name                               AS identifier,
        COALESCE(n.collection_name, '')      AS label,
        n.id::bigint                         AS surrogate_id
    FROM nfts n
    WHERE $9 = TRUE
      AND n.name IS NOT NULL
      AND n.name ILIKE '%' || $1 || '%'
    LIMIT $4
),
pool_hits AS (
    -- Pools route directly by `identifier` (hex pool_id, the natural PK).
    -- `label` shows the asset pair so the search row carries some context.
    -- For native legs (asset_*_code IS NULL) we render 'XLM' the same way
    -- the assets list / detail does.
    SELECT
        'pool'::text                AS entity_type,
        encode(lp.pool_id, 'hex')   AS identifier,
        (
            COALESCE(lp.asset_a_code, 'XLM')
            || ' / '
            || COALESCE(lp.asset_b_code, 'XLM')
        )::text                     AS label,
        NULL::bigint                AS surrogate_id
    FROM liquidity_pools lp
    WHERE $10 = TRUE
      AND $2 IS NOT NULL
      AND lp.pool_id = $2
    LIMIT $4
)
SELECT entity_type, identifier, label, surrogate_id FROM tx_hits
UNION ALL
SELECT entity_type, identifier, label, surrogate_id FROM contract_hits
UNION ALL
SELECT entity_type, identifier, label, surrogate_id FROM asset_hits
UNION ALL
SELECT entity_type, identifier, label, surrogate_id FROM account_hits
UNION ALL
SELECT entity_type, identifier, label, surrogate_id FROM nft_hits
UNION ALL
SELECT entity_type, identifier, label, surrogate_id FROM pool_hits;
