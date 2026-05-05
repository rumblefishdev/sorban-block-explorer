//! `accounts` — natural key `account_id` (StrKey), surrogate `id` excluded.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT account_id AS sk,
           account_id || '|' ||
           first_seen_ledger::text || '|' ||
           last_seen_ledger::text || '|' ||
           sequence_number::text || '|' ||
           COALESCE(home_domain, 'NULL') AS canonical
      FROM accounts
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
