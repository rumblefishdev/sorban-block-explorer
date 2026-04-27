-- Endpoint:     GET /ledgers/:sequence
-- Purpose:      Ledger detail. DB returns the header + previous/next ledger
--               sequences for navigation. The transactions-in-ledger sublist
--               is loaded by the API from the per-ledger S3 blob, NOT from
--               the partitioned `transactions` table.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.6
-- Schema:       ADR 0037
-- Data sources: DB + S3 per-ledger blob.
--               DB returns: sequence, hash, closed_at, protocol_version,
--                           transaction_count, base_fee, prev_sequence,
--                           next_sequence.
--               S3 returns: transactions[]
--                           — `s3://<bucket>/parsed_ledger_{sequence}.json`
--                           per ADR 0037 §11 bridge-column note. The DB's
--                           `ledger_sequence` is the bridge key.
-- Inputs:
--   $1  :sequence  BIGINT  the ledger sequence to fetch
-- Indexes:      ledgers PK (sequence), idx_ledgers_closed_at.
-- Notes:
--   • This is the **exception case** in task 0167's data-source boundary:
--     a detail endpoint whose embedded list lives off-DB. We deliberately do
--     NOT query the `transactions` table here — the per-ledger S3 blob is
--     the single source of truth for the embedded transactions[] array.
--   • prev/next are computed via window or a compact GREATEST/LEAST lookup.
--     Using LATERAL with LIMIT 1 yields one index-scan seek each on
--     idx_ledgers_closed_at — cheaper than a window over the whole table.
--   • The SQL returns prev/next as bare `sequence` values; the API turns
--     them into HATEOAS links. Returning NULL when at the chain head/tail
--     so the API can render "no next" / "no prev" controls correctly.

SELECT
    l.sequence,
    l.sequence                               AS ledger_sequence_s3_bridge,  -- explicit S3 key
    encode(l.hash, 'hex')                    AS hash_hex,
    l.closed_at,
    l.protocol_version,
    l.transaction_count,
    l.base_fee,
    prev.sequence                            AS prev_sequence,
    nxt.sequence                             AS next_sequence
    -- not in DB: transactions[] — S3 parsed_ledger_{ledger_sequence_s3_bridge}.json
    --            (ADR 0037 §11 / ADR 0011 layout). The bridge column is
    --            literally the same value as `sequence`, surfaced under a
    --            second alias so the API loader knows which value goes into
    --            the S3 key template.
FROM ledgers l
LEFT JOIN LATERAL (
    SELECT sequence
    FROM ledgers
    WHERE closed_at < l.closed_at
    ORDER BY closed_at DESC
    LIMIT 1
) prev ON TRUE
LEFT JOIN LATERAL (
    SELECT sequence
    FROM ledgers
    WHERE closed_at > l.closed_at
    ORDER BY closed_at ASC
    LIMIT 1
) nxt ON TRUE
WHERE l.sequence = $1;
