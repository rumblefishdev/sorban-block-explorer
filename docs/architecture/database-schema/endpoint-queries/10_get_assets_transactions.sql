-- Endpoint:     GET /assets/:id/transactions
-- Purpose:      Paginated transactions involving a given asset. The driver
--               table is `operations_appearances`, filtered by either
--               (asset_code, asset_issuer_id) for classic-form references
--               or by `contract_id` for SAC/Soroban-form references.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.9
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :asset_id            INT          assets surrogate id
--   $2  :limit               INT          page size
--   $3  :cursor_created_at   TIMESTAMPTZ  NULL on first page
--   $4  :cursor_tx_id        BIGINT       NULL on first page
-- Indexes:      assets PK (id),
--               idx_ops_app_asset    ON (asset_code, asset_issuer_id, created_at DESC) WHERE asset_code IS NOT NULL,
--               idx_ops_app_contract ON (contract_id, created_at DESC)                  WHERE contract_id IS NOT NULL,
--               transactions PK (id, created_at) — composite-FK join.
-- Notes:
--   • Two statement variants. The API picks one based on the asset's type
--     (read in the same transaction or from a small in-memory cache):
--       — Variant A: classic_credit (1) and SAC (2) referenced via the
--         classic identity tuple. Hits idx_ops_app_asset.
--       — Variant B: SAC (2) referenced via contract identity, and
--         soroban-native (3). Hits idx_ops_app_contract.
--     SAC has BOTH a classic identity and a contract identity; the API
--     SHOULD merge results from both variants (or pick one based on the
--     UX — typically classic for SAC) and dedupe on transaction_id.
--   • Native (asset_type=0) has no canonical row-level filter on
--     operations_appearances; the API may either return an empty list or
--     fall back to a global recent-transactions slice. The SQL here has
--     no native-specific path because the schema doesn't either.
--   • DISTINCT ON keeps one row per transaction even when a tx has
--     multiple ops touching the asset.
--   • Both variants use the partial indexes' predicate shape — the WHERE
--     clauses NEVER drop `asset_code IS NOT NULL` / `contract_id IS NOT NULL`,
--     so the partial indexes remain usable.

-- ============================================================================
-- A. Classic identity path: assets with (asset_code, issuer_id).
--    Used for asset_type IN (1 = classic_credit, 2 = sac).
-- ============================================================================
WITH ast AS (
    SELECT id, asset_code, issuer_id
    FROM assets
    WHERE id = $1
      AND asset_code IS NOT NULL
      AND issuer_id  IS NOT NULL
),
matched_ops AS (
    SELECT DISTINCT ON (oa.transaction_id, oa.created_at)
        oa.transaction_id,
        oa.created_at,
        oa.id AS op_appearance_id
    FROM ast
    JOIN operations_appearances oa
           ON oa.asset_code      = ast.asset_code
          AND oa.asset_issuer_id = ast.issuer_id
    WHERE oa.asset_code IS NOT NULL
      AND ($3::timestamptz IS NULL OR (oa.created_at, oa.transaction_id) < ($3, $4))
    ORDER BY oa.transaction_id, oa.created_at, oa.id
    LIMIT $2 * 4
)
SELECT
    encode(t.hash, 'hex')   AS hash_hex,
    t.ledger_sequence,
    src.account_id          AS source_account,
    t.fee_charged,
    t.successful,
    t.operation_count,
    t.has_soroban,
    ops.operation_types,    -- §6.9 reuses §6.3's "operation type" column
    t.created_at,
    t.id                    AS cursor_tx_id
FROM matched_ops m
JOIN transactions t
       ON t.id         = m.transaction_id
      AND t.created_at = m.created_at
JOIN accounts src ON src.id = t.source_id
LEFT JOIN LATERAL (
    SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
) ops ON TRUE
ORDER BY t.created_at DESC, t.id DESC
LIMIT $2;

-- @@ split @@

-- ============================================================================
-- B. Contract identity path: assets with contract_id.
--    Used for asset_type IN (2 = sac, 3 = soroban_native).
-- ============================================================================
WITH ast AS (
    SELECT id, contract_id
    FROM assets
    WHERE id = $1
      AND contract_id IS NOT NULL
),
matched_ops AS (
    SELECT DISTINCT ON (oa.transaction_id, oa.created_at)
        oa.transaction_id,
        oa.created_at,
        oa.id AS op_appearance_id
    FROM ast
    JOIN operations_appearances oa
           ON oa.contract_id = ast.contract_id
    WHERE oa.contract_id IS NOT NULL
      AND ($3::timestamptz IS NULL OR (oa.created_at, oa.transaction_id) < ($3, $4))
    ORDER BY oa.transaction_id, oa.created_at, oa.id
    LIMIT $2 * 4
)
SELECT
    encode(t.hash, 'hex')   AS hash_hex,
    t.ledger_sequence,
    src.account_id          AS source_account,
    t.fee_charged,
    t.successful,
    t.operation_count,
    t.has_soroban,
    ops.operation_types,    -- §6.9 reuses §6.3's "operation type" column
    t.created_at,
    t.id                    AS cursor_tx_id
FROM matched_ops m
JOIN transactions t
       ON t.id         = m.transaction_id
      AND t.created_at = m.created_at
JOIN accounts src ON src.id = t.source_id
LEFT JOIN LATERAL (
    SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
) ops ON TRUE
ORDER BY t.created_at DESC, t.id DESC
LIMIT $2;
