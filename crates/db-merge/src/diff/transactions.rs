//! `transactions` — natural key `(hash, created_at)`.
//! Surrogate `id` excluded; `source_id` resolved to account StrKey.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT encode(t.hash, 'hex') || '|' || t.created_at::text AS sk,
           encode(t.hash, 'hex') || '|' ||
           t.ledger_sequence::text || '|' ||
           t.application_order::text || '|' ||
           a.account_id || '|' ||
           t.fee_charged::text || '|' ||
           COALESCE(encode(t.inner_tx_hash, 'hex'), 'NULL') || '|' ||
           t.successful::text || '|' ||
           t.operation_count::text || '|' ||
           t.has_soroban::text || '|' ||
           t.parse_error::text || '|' ||
           t.created_at::text AS canonical
      FROM transactions t
      JOIN accounts a ON a.id = t.source_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
