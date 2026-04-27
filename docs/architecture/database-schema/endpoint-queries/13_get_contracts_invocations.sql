-- Endpoint:     GET /contracts/:contract_id/invocations
-- Purpose:      Paginated list of recent invocations of a contract.
--               Default ordering: most recent first.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.10
-- Schema:       ADR 0037
-- Data sources: DB-only. Function name + arguments + return value live in the
--               archive XDR (ADR 0029, ADR 0034) and are out of scope for
--               this endpoint per the documented contract — the frontend
--               only needs the appearance rows for the invocations tab.
-- Inputs:
--   $1  :contract_strkey       VARCHAR(56)  C-form contract ID
--   $2  :limit                 INT          page size
--   $3  :cursor_ledger         BIGINT       NULL on first page
--   $4  :cursor_tx_id          BIGINT       NULL on first page
--   $5  :cursor_created_at     TIMESTAMPTZ  NULL on first page
-- Indexes:      soroban_contracts UNIQUE (contract_id),
--               idx_sia_contract_ledger ON (contract_id, ledger_sequence DESC),
--               soroban_invocations_appearances PK
--                  (contract_id, transaction_id, ledger_sequence, created_at),
--               transactions PK (id, created_at) — composite-FK join.
-- Notes:
--   • Keyset on (ledger_sequence DESC, transaction_id DESC, created_at DESC).
--     The `idx_sia_contract_ledger` index is `(contract_id, ledger_sequence DESC)`
--     so the leading equality + descending range is exactly what it serves.
--   • `created_at` participates in the cursor not for ordering (ledger_sequence
--     is monotone within a partition) but for partition pruning on the next
--     page — the planner needs a literal-ish bound on the partition key.
--   • Caller StrKey via final join to accounts; some invocations have
--     caller_id NULL (top-level / non-account caller — that's what the
--     LEFT JOIN handles).

WITH ct AS (
    SELECT id FROM soroban_contracts WHERE contract_id = $1
)
SELECT
    encode(t.hash, 'hex')   AS transaction_hash_hex,
    sia.ledger_sequence,
    caller.account_id        AS caller_account,
    sia.amount,
    sia.created_at,
    t.successful,
    t.id                    AS cursor_tx_id
    -- not in DB: function_name, args, return_value — Archive XDR (ADR 0029, ADR 0034).
FROM ct
JOIN soroban_invocations_appearances sia
       ON sia.contract_id = ct.id
JOIN transactions t
       ON t.id         = sia.transaction_id
      AND t.created_at = sia.created_at
LEFT JOIN accounts caller ON caller.id = sia.caller_id
WHERE
    ($3::bigint IS NULL OR (sia.ledger_sequence, sia.transaction_id, sia.created_at) < ($3, $4, $5))
ORDER BY sia.ledger_sequence DESC, sia.transaction_id DESC, sia.created_at DESC
LIMIT $2;
