-- ============================================================================
-- account_balances_current — unpartitioned. Current state of classic trustlines.
-- Columns: account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger
-- Partial UNIQUE: uidx_abc_native (account_id) WHERE asset_type=0
--                 uidx_abc_credit (account_id, asset_code, issuer_id) WHERE asset_type<>0
-- ============================================================================
\echo '## account_balances_current'

\echo '### I1 — account_id FK valid'
SELECT COUNT(*) AS violations
FROM account_balances_current abc
LEFT JOIN accounts a ON a.id = abc.account_id
WHERE a.id IS NULL;

\echo '### I2 — issuer_id FK valid where set'
SELECT COUNT(*) AS violations
FROM account_balances_current abc
LEFT JOIN accounts a ON a.id = abc.issuer_id
WHERE abc.issuer_id IS NOT NULL AND a.id IS NULL;

\echo '### I3 — asset_type=0 (native) row has NULL asset_code/issuer_id; non-native has both'
SELECT COUNT(*) AS violations
FROM account_balances_current
WHERE NOT (
    (asset_type = 0 AND asset_code IS NULL AND issuer_id IS NULL)
 OR (asset_type <> 0 AND asset_code IS NOT NULL AND issuer_id IS NOT NULL)
);

\echo '### I4 — balance ≥ 0 (NUMERIC stored as NUMERIC(28,7))'
SELECT COUNT(*) AS violations
FROM account_balances_current
WHERE balance < 0;

\echo '### I5 — uidx_abc_native: at most one native row per account_id'
SELECT COUNT(*) AS violations
FROM (
    SELECT account_id FROM account_balances_current
    WHERE asset_type = 0
    GROUP BY account_id HAVING COUNT(*) > 1
) d;

\echo '### I6 — uidx_abc_credit: (account_id, asset_code, issuer_id) UNIQUE for non-native'
SELECT COUNT(*) AS violations
FROM (
    SELECT account_id, asset_code, issuer_id
    FROM account_balances_current
    WHERE asset_type <> 0
    GROUP BY 1,2,3 HAVING COUNT(*) > 1
) d;
