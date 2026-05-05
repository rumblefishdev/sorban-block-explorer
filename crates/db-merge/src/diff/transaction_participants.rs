//! `transaction_participants` — PK `(account_id, created_at, transaction_id)`.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT a.account_id || '|' || tp.created_at::text || '|' || encode(t.hash, 'hex') AS sk,
           a.account_id || '|' || encode(t.hash, 'hex') || '|' || tp.created_at::text AS canonical
      FROM transaction_participants tp
      JOIN transactions t ON t.id = tp.transaction_id AND t.created_at = tp.created_at
      JOIN accounts a ON a.id = tp.account_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
