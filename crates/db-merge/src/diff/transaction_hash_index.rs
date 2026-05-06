//! `transaction_hash_index` — natural key `hash` (BYTEA → hex).

pub const SQL: &str = r#"
WITH proj AS (
    SELECT encode(hash, 'hex') AS sk,
           encode(hash, 'hex') || '|' ||
           ledger_sequence::text || '|' ||
           created_at::text AS canonical
      FROM transaction_hash_index
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
