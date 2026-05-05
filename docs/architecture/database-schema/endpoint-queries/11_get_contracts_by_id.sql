-- Endpoint:     GET /contracts/:contract_id
-- Purpose:      Contract detail: header (deployer, WASM hash, type, SAC flag,
--               metadata) + lightweight stats (recent invocations and unique
--               callers in the last N days, NOT a full-history count).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.10
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :contract_strkey  VARCHAR(56)  C-form contract ID
--   $2  :stats_window     INTERVAL     stats time window (e.g. '7 days')
-- Indexes:      soroban_contracts UNIQUE (contract_id),
--               accounts PK (id) for deployer join,
--               wasm_interface_metadata PK (wasm_hash) for wasm metadata,
--               soroban_invocations_appearances PK
--                   (contract_id, transaction_id, ledger_sequence, created_at).
-- Notes:
--   • Two statements. The API runs them sequentially, threading
--     `soroban_contracts.id` from A into B.
--   • Statement A's projection of `metadata` returns the full JSONB; the API
--     decides which keys to expose. We surface the BIGINT id only as
--     `contract_pk` so statement B can use it.
--   • Statement B is the "stats" call. We deliberately bound the window
--     (`created_at >= NOW() - $2`) so this never becomes a full-history
--     scan. The header explicitly documents the chosen window so the
--     frontend can label the stats accordingly ("invocations (last 7 days)").
--   • `is_sac` is a stored BOOLEAN — surface it directly; do not derive
--     from `contract_type`.
--   • `contract_type_name()` decodes contract_type for display; WHERE
--     would compare to SMALLINT, but here we just project.

-- ============================================================================
-- A. Contract header.
-- ============================================================================
SELECT
    sc.id                              AS contract_pk,
    sc.contract_id,
    encode(sc.wasm_hash, 'hex')        AS wasm_hash_hex,
    sc.wasm_uploaded_at_ledger,
    deployer.account_id                AS deployer,
    sc.deployed_at_ledger,
    contract_type_name(sc.contract_type) AS contract_type_name,
    sc.contract_type                   AS contract_type,
    sc.is_sac
    -- Per ADR 0041 / task 0156: `metadata JSONB` replaced by typed
    -- `name VARCHAR(256)`. E11 detail response no longer projects a
    -- metadata field; `name` is consumed only by the search query
    -- (`COALESCE(sc.name, '')` in 22_get_search.sql). The detail page
    -- previously returned `{}` for every row — no information lost.
FROM soroban_contracts sc
LEFT JOIN accounts deployer ON deployer.id = sc.deployer_id
WHERE sc.contract_id = $1;

-- @@ split @@

-- ============================================================================
-- B. Recent-window stats.
--    Inputs: $1 = soroban_contracts.id (BIGINT, from A.contract_pk),
--            $2 = window interval (e.g. '7 days'::interval).
--    Indexes: soroban_invocations_appearances PK starts with contract_id; the
--             planner uses the leading equality + range on created_at.
-- ============================================================================
SELECT
    COUNT(*)                          AS recent_invocations,
    COUNT(DISTINCT sia.caller_id)     AS recent_unique_callers,
    $2::interval                      AS stats_window
FROM soroban_invocations_appearances sia
WHERE sia.contract_id = $1
  AND sia.created_at >= NOW() - $2::interval;
