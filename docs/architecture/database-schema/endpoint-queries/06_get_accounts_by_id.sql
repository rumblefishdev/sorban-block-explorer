-- Endpoint:     GET /accounts/:account_id
-- Purpose:      Account detail: header + current balances (native + credit).
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.7
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :account_strkey  VARCHAR(56)  G-form account ID (StrKey)
-- Indexes:      accounts UNIQUE (account_id),
--               account_balances_current uidx_abc_native (partial),
--               account_balances_current uidx_abc_credit (partial).
-- Notes:
--   • Two statements. The API runs them in sequence, threading
--     `account.id` from A into B.
--   • StrKey resolution happens in statement A via the UNIQUE index on
--     `accounts.account_id` (ADR 0026). Every downstream join uses the
--     `BIGINT id`.
--   • Balances are materialized in `account_balances_current`, so this
--     endpoint never scans operation history.
--   • Native row uses the `uidx_abc_native` partial unique index; credit
--     rows use `uidx_abc_credit`. Both are hit by the WHERE shape below.
--   • Issuer StrKey is surfaced via a final join to `accounts.account_id`
--     for credit rows (issuer_id IS NOT NULL); native rows (asset_type=0)
--     have NULL issuer_id and the LEFT JOIN simply yields NULL.

-- ============================================================================
-- A. Account header.
-- ============================================================================
SELECT
    a.id,
    a.account_id,
    a.first_seen_ledger,
    a.last_seen_ledger,
    a.sequence_number,
    a.home_domain
FROM accounts a
WHERE a.account_id = $1;

-- @@ split @@

-- ============================================================================
-- B. Current balances.
--    Inputs: $1 = account.id (BIGINT, from A).
--    `asset_type` is a SMALLINT enum decoded via token_asset_type_name();
--    however account_balances_current uses the classic asset_type domain
--    (native | classic_credit_alphanum4 | classic_credit_alphanum12 | pool_share),
--    so asset_type_name() is the right helper here.
-- ============================================================================
SELECT
    asset_type_name(abc.asset_type)  AS asset_type_name,
    abc.asset_type                   AS asset_type,
    abc.asset_code,
    iss.account_id                   AS asset_issuer,
    abc.balance,
    abc.last_updated_ledger
FROM account_balances_current abc
LEFT JOIN accounts iss ON iss.id = abc.issuer_id
WHERE abc.account_id = $1
ORDER BY abc.asset_type, abc.asset_code, iss.account_id;
