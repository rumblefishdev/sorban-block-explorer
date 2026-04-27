-- Endpoint:     GET /transactions
-- Purpose:      Paginated list of transactions for the global transactions
--               table. Default ordering: newest first. Optional filters:
--               source_account, contract_id, operation_type.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.3
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :limit                INT       page size (validated 1..200 in API)
--   $2  :cursor_created_at    TIMESTAMPTZ  NULL on first page
--   $3  :cursor_id            BIGINT       NULL on first page
--   $4  :source_account_id    BIGINT       NULL = no filter (resolved at API
--                                          boundary from G-StrKey via
--                                          accounts.account_id UNIQUE index;
--                                          ADR 0026)
--   $5  :contract_id          BIGINT       NULL = no filter (resolved from
--                                          C-StrKey; ADR 0030). When set, the
--                                          API MUST call statement B below;
--                                          statement A ignores this param.
--   $6  :op_type              SMALLINT     NULL = no filter (op_type_name
--                                          enum SMALLINT; ADR 0031). When set,
--                                          API MUST call statement B below.
-- Indexes:      Statement A: idx_tx_source_created (when source filter set),
--                            transactions PK (created_at, id) keyset.
--               Statement B: idx_ops_app_contract OR idx_ops_app_type
--                            (depending on which filter is dominant), then
--                            transactions PK on (transaction_id, created_at).
-- INDEX GAP: ADR 0037 has no global `(created_at DESC, id DESC)` index on
--             `transactions`. Without source / has_soroban filters,
--             statement A relies on partition-append + per-partition seq scan
--             ordered at the planner's discretion — fast for first-page in
--             the latest partition (LIMIT short-circuits) but degrades on
--             deep pagination. Add the index in task **0132** if the no-
--             filter case becomes hot. The other statement-A paths (with
--             source filter, or filter via has_soroban) are covered by
--             idx_tx_source_created / idx_tx_has_soroban respectively.
-- Notes:
--   • Two statements. The API picks one at request time:
--       — Statement A: no contract / op_type filter (the common case).
--       — Statement B: contract_id and/or op_type filter set. Driving from
--         operations_appearances is correct because those filters have no
--         covering index on `transactions` itself.
--   • Both use keyset pagination on (created_at DESC, id DESC). Cursor is the
--     pair from the last row of the previous page; first page passes NULLs.
--     Row-value comparison `(t.created_at, t.id) < ($2, $3)` lets the planner
--     walk the PK in descending order with a single seek.
--   • Source StrKey in the response comes from a final join back to
--     `accounts.account_id`; never project the raw BIGINT id.
--   • op_type is decoded via op_type_name(SMALLINT) in the projection only;
--     never in WHERE.
--   • For statement B the DISTINCT ON keeps one row per transaction even if
--     the tx has multiple matching ops.
--   • Per-row "primary op" preview (StellarChain-style FROM / TO / AMOUNT):
--     a second LATERAL picks the FIRST op of each tx (`ORDER BY oa.id LIMIT 1`)
--     and projects its from/to/interacted_with/amount/asset_code plus a
--     coarse `primary_op_method` bucket. For multi-op tx (operation_count > 1)
--     the frontend shows the primary op + a "+N more" indicator and links to
--     /transactions/:hash for the full list. Cost: one extra PK seek per
--     row, partition-pruned via the same composite FK as op_types LATERAL.
--   • `primary_op_from` falls back to tx-level `source_account` when the
--     op-level source is NULL (per Stellar protocol, an op without explicit
--     source uses the tx source).
--   • `primary_op_interacted_with` is a COALESCE across the three FK
--     targets in `operations_appearances` (contract_id / pool_id /
--     destination_id) — exactly one is non-NULL for op types that have a
--     "to" semantically; for offers all three are NULL and the frontend
--     shows the asset_code alone (the second leg of the trading pair lives
--     in archive XDR per ADR 0029, not here). The companion column
--     `primary_op_interacted_with_kind` ('contract' / 'pool' / 'account' /
--     NULL) tells the frontend how to render the value.

-- ============================================================================
-- Statement A — no contract / op_type filter (default path)
-- ============================================================================
SELECT
    encode(t.hash, 'hex')         AS hash_hex,
    t.ledger_sequence,
    t.application_order,
    src.account_id                AS source_account,
    t.fee_charged,
    encode(t.inner_tx_hash, 'hex') AS inner_tx_hash_hex,
    t.successful,
    t.operation_count,
    t.has_soroban,
    ops.operation_types,           -- frontend §6.3 "operation type" column

    -- Primary-op preview (one row per tx; first op by oa.id).
    op_type_name(pop.type)         AS primary_op_type,
    CASE
        WHEN pop.type IN (1, 2, 13)         THEN 'payment'
        WHEN pop.type IN (3, 4, 12)         THEN 'offer'
        WHEN pop.type IN (24, 25, 26)       THEN 'contract'
        WHEN pop.type IN (22, 23)           THEN 'liquidity_pool'
        WHEN pop.type IN (6, 7, 19, 21)     THEN 'trust'
        WHEN pop.type IN (14, 15, 20)       THEN 'claimable_balance'
        WHEN pop.type IN (16, 17, 18)       THEN 'sponsorship'
        WHEN pop.type IS NULL               THEN NULL
        ELSE 'account'
    END                            AS primary_op_method,
    COALESCE(op_src.account_id,
             src.account_id)       AS primary_op_from,
    COALESCE(
        op_ctr.contract_id,
        encode(pop.pool_id, 'hex'),
        op_dst.account_id
    )                              AS primary_op_interacted_with,
    CASE
        WHEN op_ctr.contract_id IS NOT NULL THEN 'contract'
        WHEN pop.pool_id        IS NOT NULL THEN 'pool'
        WHEN op_dst.account_id  IS NOT NULL THEN 'account'
        ELSE NULL
    END                            AS primary_op_interacted_with_kind,
    pop.amount                     AS primary_op_amount,
    pop.asset_code                 AS primary_op_asset_code,
    op_iss.account_id              AS primary_op_asset_issuer,

    t.created_at,
    t.id                          AS cursor_id  -- echo for next-page cursor
FROM transactions t
JOIN accounts src ON src.id = t.source_id
LEFT JOIN LATERAL (
    -- All distinct op types present in this tx, decoded for display.
    -- LATERAL keeps the lookup partition-pruned via composite FK
    -- `(transaction_id, created_at)`. Cost per row: one PK seek into the
    -- single matching partition, returning at most `operation_count` rows.
    SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
) ops ON TRUE
LEFT JOIN LATERAL (
    -- Primary op preview: first op of the tx, identified by smallest oa.id
    -- (BIGSERIAL is monotone with ingestion order, which equals op application
    -- order within a single tx). One PK seek + LIMIT 1 per row.
    SELECT
        oa.type,
        oa.source_id,
        oa.destination_id,
        oa.contract_id,
        oa.pool_id,
        oa.asset_code,
        oa.asset_issuer_id,
        oa.amount
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
    ORDER BY oa.id
    LIMIT 1
) pop ON TRUE
LEFT JOIN accounts          op_src ON op_src.id = pop.source_id
LEFT JOIN accounts          op_dst ON op_dst.id = pop.destination_id
LEFT JOIN soroban_contracts op_ctr ON op_ctr.id = pop.contract_id
LEFT JOIN accounts          op_iss ON op_iss.id = pop.asset_issuer_id
WHERE
    ($2::timestamptz IS NULL OR (t.created_at, t.id) < ($2, $3))
    AND ($4::bigint IS NULL OR t.source_id = $4)
ORDER BY t.created_at DESC, t.id DESC
LIMIT $1;

-- @@ split @@

-- ============================================================================
-- Statement B — contract_id and/or op_type filter (drives from ops index)
-- ============================================================================
WITH matched_ops AS (
    -- Pick newest matches first so the LIMIT $1 * 4 truncates the *tail*,
    -- not an arbitrary middle slice. DISTINCT ON's leading expressions
    -- must match the leading ORDER BY columns, so we put `(created_at,
    -- transaction_id)` in BOTH and rely on DESC-ordered partial indexes
    -- `(contract_id, created_at DESC)` / `(type, created_at DESC)` for
    -- the descending walk. `oa.id` is just a deterministic tie-breaker
    -- inside one (created_at, transaction_id) pair (multi-op tx where
    -- multiple ops share the filter).
    SELECT DISTINCT ON (oa.created_at, oa.transaction_id)
        oa.transaction_id,
        oa.created_at
    FROM operations_appearances oa
    WHERE
        ($2::timestamptz IS NULL OR (oa.created_at, oa.transaction_id) < ($2, $3))
        AND ($5::bigint   IS NULL OR oa.contract_id = $5)
        AND ($6::smallint IS NULL OR oa.type        = $6)
    ORDER BY oa.created_at DESC, oa.transaction_id DESC, oa.id
    -- LIMIT is generous (limit * fan-out factor) so the final
    -- `JOIN transactions` + outer LIMIT $1 still has enough candidates
    -- after fan-out across multi-op tx.
    LIMIT $1 * 4
)
SELECT
    encode(t.hash, 'hex')         AS hash_hex,
    t.ledger_sequence,
    t.application_order,
    src.account_id                AS source_account,
    t.fee_charged,
    encode(t.inner_tx_hash, 'hex') AS inner_tx_hash_hex,
    t.successful,
    t.operation_count,
    t.has_soroban,
    ops.operation_types,           -- ALL op types in tx, not just matched

    -- Primary-op preview (same shape as statement A).
    op_type_name(pop.type)         AS primary_op_type,
    CASE
        WHEN pop.type IN (1, 2, 13)         THEN 'payment'
        WHEN pop.type IN (3, 4, 12)         THEN 'offer'
        WHEN pop.type IN (24, 25, 26)       THEN 'contract'
        WHEN pop.type IN (22, 23)           THEN 'liquidity_pool'
        WHEN pop.type IN (6, 7, 19, 21)     THEN 'trust'
        WHEN pop.type IN (14, 15, 20)       THEN 'claimable_balance'
        WHEN pop.type IN (16, 17, 18)       THEN 'sponsorship'
        WHEN pop.type IS NULL               THEN NULL
        ELSE 'account'
    END                            AS primary_op_method,
    COALESCE(op_src.account_id,
             src.account_id)       AS primary_op_from,
    COALESCE(
        op_ctr.contract_id,
        encode(pop.pool_id, 'hex'),
        op_dst.account_id
    )                              AS primary_op_interacted_with,
    CASE
        WHEN op_ctr.contract_id IS NOT NULL THEN 'contract'
        WHEN pop.pool_id        IS NOT NULL THEN 'pool'
        WHEN op_dst.account_id  IS NOT NULL THEN 'account'
        ELSE NULL
    END                            AS primary_op_interacted_with_kind,
    pop.amount                     AS primary_op_amount,
    pop.asset_code                 AS primary_op_asset_code,
    op_iss.account_id              AS primary_op_asset_issuer,

    t.created_at,
    t.id                          AS cursor_id
FROM matched_ops m
JOIN transactions t
     ON t.id = m.transaction_id
    AND t.created_at = m.created_at  -- composite FK + partition prune
JOIN accounts src ON src.id = t.source_id
LEFT JOIN LATERAL (
    SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
) ops ON TRUE
LEFT JOIN LATERAL (
    SELECT
        oa.type,
        oa.source_id,
        oa.destination_id,
        oa.contract_id,
        oa.pool_id,
        oa.asset_code,
        oa.asset_issuer_id,
        oa.amount
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
    ORDER BY oa.id
    LIMIT 1
) pop ON TRUE
LEFT JOIN accounts          op_src ON op_src.id = pop.source_id
LEFT JOIN accounts          op_dst ON op_dst.id = pop.destination_id
LEFT JOIN soroban_contracts op_ctr ON op_ctr.id = pop.contract_id
LEFT JOIN accounts          op_iss ON op_iss.id = pop.asset_issuer_id
WHERE ($4::bigint IS NULL OR t.source_id = $4)
  AND ($2::timestamptz IS NULL OR (t.created_at, t.id) < ($2, $3))
ORDER BY t.created_at DESC, t.id DESC
LIMIT $1;
