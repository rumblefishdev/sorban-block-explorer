//! `lp_positions` — PK `(pool_id, account_id)`.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT encode(lp.pool_id, 'hex') || '|' || a.account_id AS sk,
           encode(lp.pool_id, 'hex') || '|' ||
           a.account_id || '|' ||
           lp.shares::text || '|' ||
           lp.first_deposit_ledger::text || '|' ||
           lp.last_updated_ledger::text AS canonical
      FROM lp_positions lp
      JOIN accounts a ON a.id = lp.account_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
