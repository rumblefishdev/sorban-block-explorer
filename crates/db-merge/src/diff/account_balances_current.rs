//! `account_balances_current` — natural key path-dependent.
//! Native: `account_id`. Credit: `(account_id, asset_code, issuer_strkey)`.
//! Combined sort key handles both.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT a.account_id || '|' || abc.asset_type::text || '|' ||
           COALESCE(abc.asset_code, '') || '|' ||
           COALESCE(i.account_id, '') AS sk,
           a.account_id || '|' ||
           abc.asset_type::text || '|' ||
           COALESCE(abc.asset_code, 'NULL') || '|' ||
           COALESCE(i.account_id, 'NULL') || '|' ||
           abc.balance::text || '|' ||
           abc.last_updated_ledger::text AS canonical
      FROM account_balances_current abc
      JOIN accounts a ON a.id = abc.account_id
      LEFT JOIN accounts i ON i.id = abc.issuer_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
