-- Endpoint:     GET /accounts/:account_id/transactions
-- Purpose:      Paginated transactions involving a given account (as source
--               OR as a participant). Default ordering: newest first.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.7
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :account_strkey      VARCHAR(56)  G-form account ID (StrKey)
--   $2  :limit               INT          page size
--   $3  :cursor_created_at   TIMESTAMPTZ  NULL on first page
--   $4  :cursor_tx_id        BIGINT       NULL on first page
-- Indexes:      accounts UNIQUE (account_id),
--               transaction_participants PK (account_id, created_at, transaction_id) — used for keyset,
--               transactions PK (id, created_at) — composite-FK lookup.
-- Notes:
--   • Single statement. The CTE `acc` resolves the StrKey to a BIGINT id
--     in one indexed lookup; the planner inlines this and uses the natural
--     PK ordering of `transaction_participants` for the keyset walk.
--   • The PK ordering on transaction_participants is
--     `(account_id, created_at, transaction_id)` — exactly what we need to
--     scan in `(created_at DESC, transaction_id DESC)` order for one
--     account, with `created_at` already partition-pruning the underlying
--     partitions.
--   • The `transactions` lookup uses the composite (id, created_at) FK so
--     it stays inside one partition per row.
--   • `transaction_participants` includes the source account too (per
--     ingestion contract — see ADR 0020 / task 0163), so this endpoint
--     does NOT need a UNION with `transactions.source_id`.

WITH acc AS (
    SELECT id FROM accounts WHERE account_id = $1
)
SELECT
    encode(t.hash, 'hex')          AS hash_hex,
    t.ledger_sequence,
    t.application_order,
    src.account_id                 AS source_account,
    t.fee_charged,
    t.successful,
    t.operation_count,
    t.has_soroban,
    ops.operation_types,           -- §6.7 reuses §6.3's "operation type" column
    t.created_at,
    t.id                           AS cursor_tx_id
FROM acc
JOIN transaction_participants tp
       ON tp.account_id = acc.id
JOIN transactions t
       ON t.id         = tp.transaction_id
      AND t.created_at = tp.created_at
JOIN accounts src ON src.id = t.source_id
LEFT JOIN LATERAL (
    SELECT array_agg(DISTINCT op_type_name(oa.type) ORDER BY op_type_name(oa.type)) AS operation_types
    FROM operations_appearances oa
    WHERE oa.transaction_id = t.id
      AND oa.created_at     = t.created_at
) ops ON TRUE
WHERE
    ($3::timestamptz IS NULL OR (tp.created_at, tp.transaction_id) < ($3, $4))
ORDER BY tp.created_at DESC, tp.transaction_id DESC
LIMIT $2;
