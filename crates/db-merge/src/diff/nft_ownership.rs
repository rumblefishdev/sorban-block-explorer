//! `nft_ownership` — PK `(nft_id, created_at, ledger_sequence, event_order)`.
//! `nft_id` resolved to `(contract_strkey, token_id)`.

pub const SQL: &str = r#"
WITH proj AS (
    SELECT sc.contract_id || '|' || n.token_id || '|' ||
           no.created_at::text || '|' || no.ledger_sequence::text || '|' ||
           no.event_order::text AS sk,
           sc.contract_id || '|' ||
           n.token_id || '|' ||
           encode(t.hash, 'hex') || '|' ||
           COALESCE(a.account_id, 'NULL') || '|' ||
           no.event_type::text || '|' ||
           no.ledger_sequence::text || '|' ||
           no.event_order::text || '|' ||
           no.created_at::text AS canonical
      FROM nft_ownership no
      JOIN nfts n ON n.id = no.nft_id
      JOIN soroban_contracts sc ON sc.id = n.contract_id
      JOIN transactions t ON t.id = no.transaction_id AND t.created_at = no.created_at
      LEFT JOIN accounts a ON a.id = no.owner_id
)
SELECT md5(string_agg(canonical, chr(31) ORDER BY sk)) AS hash,
       count(*)::bigint AS rows
  FROM proj
"#;
