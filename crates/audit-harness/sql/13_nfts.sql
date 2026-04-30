-- ============================================================================
-- nfts — unpartitioned. NFT registry.
-- Columns: id, contract_id, token_id, collection_name, name, media_url,
--          metadata, minted_at_ledger, current_owner_id, current_owner_ledger
-- ============================================================================
\echo '## nfts'

\echo '### I1 — (contract_id, token_id) UNIQUE'
SELECT COUNT(*) AS violations
FROM (
    SELECT contract_id, token_id FROM nfts
    GROUP BY 1,2 HAVING COUNT(*) > 1
) d;

\echo '### I2 — contract_id FK to soroban_contracts valid'
SELECT COUNT(*) AS violations
FROM nfts n
LEFT JOIN soroban_contracts c ON c.id = n.contract_id
WHERE c.id IS NULL;

\echo '### I3 — current_owner_id FK to accounts valid where set'
SELECT COUNT(*) AS violations
FROM nfts n
LEFT JOIN accounts a ON a.id = n.current_owner_id
WHERE n.current_owner_id IS NOT NULL AND a.id IS NULL;

\echo '### I4 — minted_at_ledger ≤ current_owner_ledger (monotonic, mint precedes any transfer)'
SELECT COUNT(*) AS violations
FROM nfts
WHERE minted_at_ledger IS NOT NULL
  AND current_owner_ledger IS NOT NULL
  AND minted_at_ledger > current_owner_ledger;

\echo '### I5 — last nft_ownership row per nft → matches nfts.current_owner_id (mat-view consistency)'
WITH last_event AS (
    SELECT DISTINCT ON (nft_id) nft_id, owner_id, ledger_sequence, event_order
    FROM nft_ownership
    ORDER BY nft_id, ledger_sequence DESC, event_order DESC
)
SELECT COUNT(*) AS violations,
       (SELECT array_agg(le.nft_id) FROM (
           SELECT le.nft_id FROM last_event le
           JOIN nfts n ON n.id = le.nft_id
           WHERE n.current_owner_id IS DISTINCT FROM le.owner_id
           ORDER BY le.nft_id LIMIT 5
       ) s) AS sample
FROM last_event le
JOIN nfts n ON n.id = le.nft_id
WHERE n.current_owner_id IS DISTINCT FROM le.owner_id;
