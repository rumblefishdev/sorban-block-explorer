-- ============================================================================
-- nft_ownership — partitioned by RANGE (created_at). Mint/transfer/burn history.
-- Columns: nft_id, transaction_id, owner_id, event_type, ledger_sequence, event_order, created_at
-- ============================================================================
\echo '## nft_ownership'

\echo '### I1 — nft_id FK to nfts valid'
SELECT COUNT(*) AS violations
FROM nft_ownership no
LEFT JOIN nfts n ON n.id = no.nft_id
WHERE n.id IS NULL;

\echo '### I2 — transaction_id FK valid'
SELECT COUNT(*) AS violations
FROM nft_ownership no
LEFT JOIN transactions t ON t.id = no.transaction_id AND t.created_at = no.created_at
WHERE t.id IS NULL;

\echo '### I3 — owner_id FK to accounts valid where set'
-- owner_id may be NULL for burn events
SELECT COUNT(*) AS violations
FROM nft_ownership no
LEFT JOIN accounts a ON a.id = no.owner_id
WHERE no.owner_id IS NOT NULL AND a.id IS NULL;

\echo '### I4 — event_type SMALLINT in valid range (mint/transfer/burn enum)'
SELECT COUNT(*) AS violations
FROM nft_ownership
WHERE event_type < 0 OR event_type > 10;

\echo '### I5 — first event per nft is a mint (event_type denoting mint)'
-- Sanity: for each NFT, the chronologically-first event in nft_ownership should be a mint.
-- This catches missing-mint-event extraction bugs.
WITH first_event AS (
    SELECT DISTINCT ON (nft_id) nft_id, event_type, ledger_sequence, event_order
    FROM nft_ownership
    ORDER BY nft_id, ledger_sequence ASC, event_order ASC
)
SELECT COUNT(*) AS informational_only
FROM first_event
WHERE event_type <> 1;  -- assuming mint=1; revisit if enum changes

\echo '### I6 — event_order non-negative within ledger'
SELECT COUNT(*) AS violations
FROM nft_ownership WHERE event_order < 0;
