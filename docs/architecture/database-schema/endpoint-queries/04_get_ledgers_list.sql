-- Endpoint:     GET /ledgers
-- Purpose:      Paginated list of ledgers for the chain history browser.
--               Default ordering: newest closed_at first.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.5
-- Schema:       ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :limit              INT          page size (validated 1..200 in API)
--   $2  :cursor_closed_at   TIMESTAMPTZ  NULL on first page
--   $3  :cursor_sequence    BIGINT       NULL on first page
-- Indexes:      idx_ledgers_closed_at (closed_at DESC).
-- Notes:
--   • Keyset on (closed_at DESC, sequence DESC). The closed_at index alone
--     would be enough for ordering, but pairing with `sequence` makes the
--     cursor totally ordered even in the (very rare) tie case.
--   • Both filter and order touch closed_at directly — no function wrapping,
--     no expression on the indexed column, so the index is fully usable.

SELECT
    l.sequence,
    encode(l.hash, 'hex')   AS hash_hex,
    l.closed_at,
    l.protocol_version,
    l.transaction_count,
    l.base_fee
FROM ledgers l
WHERE $2::timestamptz IS NULL
   OR (l.closed_at, l.sequence) < ($2, $3)
ORDER BY l.closed_at DESC, l.sequence DESC
LIMIT $1;
