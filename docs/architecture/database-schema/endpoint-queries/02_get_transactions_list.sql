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
--                                          API MUST call statement B.
--   $6  :op_type              SMALLINT     NULL = no filter (op_type_name
--                                          enum SMALLINT; ADR 0031). When set
--                                          alone (without $5), API MUST call
--                                          statement C.
-- Indexes:      Statement A: idx_tx_source_created (when source filter set),
--                            transactions PK keyset.
--                            Per-row LATERAL into appearance tables uses:
--                              uq_ops_app_identity     (transaction_id leading)
--                              idx_sea_transaction     (transaction_id, created_at DESC)
--                              idx_sia_transaction     (transaction_id)
--               Statement B: idx_ops_app_contract     (contract_id, created_at DESC)
--                            idx_sia_contract_ledger  (contract_id, ledger_sequence DESC)
--                            idx_sea_contract_ledger  (contract_id, ledger_sequence DESC)
--                            UNION-merged into a small (transaction_id, created_at)
--                            candidate set, then PK-joined to transactions.
--                            idx_ops_app_type only used for the optional
--                            EXISTS post-filter when $6 is also set.
--               Statement C: idx_ops_app_type (type, created_at DESC) keyset,
--                            then PK-joined to transactions.
-- INDEX GAP (Statement A): ADR 0037 has no global
--             `(created_at DESC, id DESC)` index on `transactions`.
--             Statement A relies on partition-append + per-partition seq
--             scan ordered at the planner's discretion — fast for first-
--             page in the latest partition (LIMIT short-circuits) but
--             degrades on deep pagination. Add the index in task **0132**
--             if the no-filter case becomes hot.
-- INDEX GAP (Statement B): the soroban_invocations_appearances and
--             soroban_events_appearances UNION branches keyset-filter
--             and ORDER BY `(created_at, transaction_id)` while the
--             contract-leading partial indexes lead with
--             `(contract_id, ledger_sequence DESC)` —
--             `idx_sia_contract_ledger` has no `created_at`,
--             `idx_sea_contract_ledger` has it but in third position
--             (after `ledger_sequence`). On rare contracts the planner
--             falls through to the composite PK and serves the few
--             matches in sub-ms (verified 0.287 ms on a 100-ledger
--             sample), but on a popular contract with millions of
--             rows mainnet-side the cursor walk forces a sort step.
--             Two fixes are equivalent and both belong in task **0132**:
--             (a) add aligned indexes
--             `(contract_id, created_at DESC, transaction_id DESC)`
--             on both tables; (b) switch the UNION branches to keyset
--             on `ledger_sequence` (uses existing indexes natively but
--             complicates the API's cursor encoding by introducing
--             two cursor flavors). Owner's call. Until then, plan
--             quality on those branches scales linearly with per-
--             contract row count.
-- Notes:
--   • Three statements. The API picks one at request time:
--       — Statement A: no contract / op_type filter (the common case).
--       — Statement B: contract_id filter set (with or without op_type).
--         The contract match is "broad" — $5 is considered touched if it
--         appears in any of the three Soroban appearance tables: root op
--         (`operations_appearances`), call-tree node
--         (`soroban_invocations_appearances`), or event emitter
--         (`soroban_events_appearances`). Aligns with stellar.expert
--         semantics so a search for an AMM token finds tx where the token
--         was nested-called by a router, not just direct invocations.
--       — Statement C: op_type filter only. Drives from idx_ops_app_type.
--   • All three use keyset pagination on (created_at DESC, id DESC). Cursor
--     is the pair from the last row of the previous page; first page passes
--     NULLs. Row-value comparison `(t.created_at, t.id) < ($2, $3)` lets the
--     planner walk in descending order with a single seek.
--   • Source StrKey in the response comes from a final join back to
--     `accounts.account_id`; never project the raw BIGINT id.
--   • op_type is decoded via op_type_name(SMALLINT) in the projection only;
--     never in WHERE.
--   • `contract_ids[]` is built from a UNION over operations_appearances +
--     soroban_invocations_appearances + soroban_events_appearances. NULL on
--     the row for non-Soroban tx (no contracts touched). The same UNION is
--     used both for projection and (in statement B) for the search filter,
--     so a tx returned for `contract=$X` always has $X in its
--     `contract_ids[]`.
--   • The frontend list (frontend-overview §6.3) shows hash / ledger /
--     source / status / fee / operation_count / operation_types[]. The
--     `contract_ids[]` array supports a future "Touched contracts" column
--     (badge cluster) and aligns the search filter with the displayed set.
--     No per-tx "primary op" FROM/TO/AMOUNT preview is exposed: per-op
--     stroop value lives in archive XDR (ADR 0029) and the equivalent DB
--     column on `operations_appearances` is a fold count, not a value
--     (task 0163 / 0169).

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
    ctr.contract_ids,              -- C-StrKeys touched anywhere in tx (UNION 3 appearance tables)

    t.created_at,
    t.id                          AS cursor_id  -- echo for next-page cursor
FROM transactions t
JOIN accounts src ON src.id = t.source_id
LEFT JOIN LATERAL (
    SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
) ops ON TRUE
LEFT JOIN LATERAL (
    -- All distinct contracts touched by this tx, across 3 appearance tables.
    -- See header notes for the union semantics.
    SELECT array_agg(DISTINCT sc.contract_id ORDER BY sc.contract_id) AS contract_ids
    FROM (
        SELECT contract_id FROM operations_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
          AND contract_id IS NOT NULL
        UNION
        SELECT contract_id FROM soroban_invocations_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
        UNION
        SELECT contract_id FROM soroban_events_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
    ) all_ctr
    JOIN soroban_contracts sc ON sc.id = all_ctr.contract_id
) ctr ON TRUE
WHERE
    ($2::timestamptz IS NULL OR (t.created_at, t.id) < ($2, $3))
    AND ($4::bigint IS NULL OR t.source_id = $4)
ORDER BY t.created_at DESC, t.id DESC
LIMIT $1;

-- @@ split @@

-- ============================================================================
-- Statement B — contract filter set (with or without op_type)
-- ============================================================================
-- The contract match is broadened to all three appearance tables. We build a
-- small candidate set via UNION of the three contract-leading partial
-- indexes, ORDER + LIMIT it (over-fetch by 4× to handle multi-table fan-out
-- before final tx-level dedup), PK-join to transactions, and apply the
-- optional op_type filter as a post-EXISTS check.
WITH matched_tx AS (
    SELECT DISTINCT created_at, transaction_id
    FROM (
        -- Root invocation: tx where the InvokeHostFunction targeted $5.
        SELECT created_at, transaction_id FROM operations_appearances
        WHERE contract_id = $5
          AND ($2::timestamptz IS NULL OR (created_at, transaction_id) < ($2, $3))
        UNION
        -- Call-tree node: tx whose Soroban execution invoked $5 (nested or root).
        SELECT created_at, transaction_id FROM soroban_invocations_appearances
        WHERE contract_id = $5
          AND ($2::timestamptz IS NULL OR (created_at, transaction_id) < ($2, $3))
        UNION
        -- Event emitter: tx where $5 emitted at least one Soroban event.
        SELECT created_at, transaction_id FROM soroban_events_appearances
        WHERE contract_id = $5
          AND ($2::timestamptz IS NULL OR (created_at, transaction_id) < ($2, $3))
    ) u
    ORDER BY created_at DESC, transaction_id DESC
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
    ops.operation_types,           -- ALL op types in tx (not just $6 if set)
    ctr.contract_ids,              -- ALL contracts touched (always includes $5)

    t.created_at,
    t.id                          AS cursor_id
FROM matched_tx m
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
    SELECT array_agg(DISTINCT sc.contract_id ORDER BY sc.contract_id) AS contract_ids
    FROM (
        SELECT contract_id FROM operations_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
          AND contract_id IS NOT NULL
        UNION
        SELECT contract_id FROM soroban_invocations_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
        UNION
        SELECT contract_id FROM soroban_events_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
    ) all_ctr
    JOIN soroban_contracts sc ON sc.id = all_ctr.contract_id
) ctr ON TRUE
WHERE
    ($4::bigint IS NULL OR t.source_id = $4)
    -- op_type post-filter (when $6 also set): the tx must contain at least
    -- one operation of this type. Cheap per-row EXISTS via composite FK.
    AND ($6::smallint IS NULL OR EXISTS (
        SELECT 1 FROM operations_appearances oa2
        WHERE oa2.transaction_id = t.id
          AND oa2.created_at     = t.created_at
          AND oa2.type           = $6
    ))
ORDER BY t.created_at DESC, t.id DESC
LIMIT $1;

-- @@ split @@

-- ============================================================================
-- Statement C — op_type filter only (no contract filter)
-- ============================================================================
-- Original Statement B pattern preserved for the op_type-only case: drive
-- from idx_ops_app_type to walk the appearance index ordered by
-- (created_at DESC, transaction_id DESC), DISTINCT ON to dedupe multi-op tx,
-- then PK-join transactions.
WITH matched_ops AS (
    SELECT DISTINCT ON (oa.created_at, oa.transaction_id)
        oa.transaction_id,
        oa.created_at
    FROM operations_appearances oa
    WHERE
        ($2::timestamptz IS NULL OR (oa.created_at, oa.transaction_id) < ($2, $3))
        AND oa.type = $6
    ORDER BY oa.created_at DESC, oa.transaction_id DESC, oa.id
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
    ops.operation_types,
    ctr.contract_ids,

    t.created_at,
    t.id                          AS cursor_id
FROM matched_ops m
JOIN transactions t
     ON t.id = m.transaction_id
    AND t.created_at = m.created_at
JOIN accounts src ON src.id = t.source_id
LEFT JOIN LATERAL (
    SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
) ops ON TRUE
LEFT JOIN LATERAL (
    SELECT array_agg(DISTINCT sc.contract_id ORDER BY sc.contract_id) AS contract_ids
    FROM (
        SELECT contract_id FROM operations_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
          AND contract_id IS NOT NULL
        UNION
        SELECT contract_id FROM soroban_invocations_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
        UNION
        SELECT contract_id FROM soroban_events_appearances
        WHERE transaction_id = t.id
          AND created_at     = t.created_at
    ) all_ctr
    JOIN soroban_contracts sc ON sc.id = all_ctr.contract_id
) ctr ON TRUE
WHERE
    ($4::bigint IS NULL OR t.source_id = $4)
    AND ($2::timestamptz IS NULL OR (t.created_at, t.id) < ($2, $3))
ORDER BY t.created_at DESC, t.id DESC
LIMIT $1;
