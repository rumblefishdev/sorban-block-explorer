//! `ledgers` — natural key `sequence`, no surrogate, no FKs.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT sequence AS sk,
           sequence::text || '|' ||
           encode(hash, 'hex') || '|' ||
           closed_at::text || '|' ||
           protocol_version::text || '|' ||
           transaction_count::text || '|' ||
           base_fee::text AS canonical
      FROM ledgers
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
