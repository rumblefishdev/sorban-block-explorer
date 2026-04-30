-- Endpoint:     GET /nfts/:id/transfers
-- Purpose:      Paginated transfer/ownership history for a single NFT.
--               Default ordering: most recent first.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.12
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :nft_id                INT          NFT surrogate id
--   $2  :limit                 INT          page size
--   $3  :cursor_created_at     TIMESTAMPTZ  NULL on first page
--   $4  :cursor_ledger         BIGINT       NULL on first page
--   $5  :cursor_event_order    SMALLINT     NULL on first page
-- Indexes:      nfts PK (id),
--               nft_ownership PK (nft_id, created_at, ledger_sequence, event_order)
--                  — the natural keyset.
-- Notes:
--   • The PK on `nft_ownership` is exactly the ordering we want
--     `(nft_id =, created_at DESC, ledger_sequence DESC, event_order DESC)`,
--     which means the keyset walk is one index seek per page.
--   • event_type decoded via `nft_event_type_name(SMALLINT)`. The
--     SMALLINT itself is also surfaced for clients that want the raw
--     enum.
--   • `owner_id` is NULL on a burn (per ADR 0037 §13); LEFT JOIN handles it.
--   • The transaction-hash join uses the composite FK
--     `(transaction_id, created_at)` so it stays partition-pruned.
--   • `from_owner` synthesis: `nft_ownership.owner_id` stores ONLY the new
--     owner after each event — there is no `from_owner_id` column. Frontend
--     §6.12 requires "Alice → Bob" in the transfer-history table, so we
--     reconstruct from-owner with a window function. With the result set
--     ordered DESC (newest first), the OLDER event sits at the FOLLOWING
--     window position, so the previous owner is `LEAD(owner)` (not LAG).
--     The mint row (oldest event, last in DESC window) yields NULL because
--     LEAD has no following row, which renders as "(mint)" on the frontend.
--     Earlier drafts of this query used LAG; that was incorrect — LAG on a
--     DESC window pulls the NEWER row's owner, which is the next-owner not
--     the previous-owner. Pagination boundary: the API caller fetches
--     `LIMIT = page_size + 1` (peek-for-has-more pattern). The +1 peek row
--     is included in the window-function input, so LEAD on the LAST KEPT
--     row (index `page_size - 1`) reads the peek row's owner — i.e. the
--     correct previous-owner across the page cut. The peek row itself is
--     dropped client-of-SQL by `finalize_page`. The only NULL `from_owner`
--     remaining is the actual mint event (oldest row, no row below it in
--     either the page or the table), which is exactly the intended
--     "(mint)" rendering. No API-side stitching required.

SELECT
    no.created_at,
    no.ledger_sequence,
    no.event_order,
    nft_event_type_name(no.event_type)  AS event_type_name,
    no.event_type                       AS event_type,
    LEAD(own.account_id) OVER (
        PARTITION BY no.nft_id
        ORDER BY no.created_at DESC,
                 no.ledger_sequence DESC,
                 no.event_order DESC
    )                                   AS from_owner,
    own.account_id                      AS to_owner,
    encode(t.hash, 'hex')               AS transaction_hash_hex
FROM nft_ownership no
LEFT JOIN accounts     own ON own.id = no.owner_id
JOIN      transactions t
       ON t.id         = no.transaction_id
      AND t.created_at = no.created_at
WHERE no.nft_id = $1
  AND ($3::timestamptz IS NULL
       OR (no.created_at, no.ledger_sequence, no.event_order) < ($3, $4, $5))
ORDER BY no.created_at DESC, no.ledger_sequence DESC, no.event_order DESC
LIMIT $2;
