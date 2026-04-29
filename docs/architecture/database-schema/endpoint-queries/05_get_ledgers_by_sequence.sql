-- Endpoint:     GET /ledgers/:sequence
-- Purpose:      Ledger detail — header row + prev/next navigation + embedded
--               paginated transactions[] for the ledger.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.6
-- Schema:       ADR 0037
-- Data sources: DB-only.
--               Statement A: ledgers header + prev_sequence + next_sequence.
--               Statement B: transactions in this ledger, partition-pruned via
--                            the ledger's `closed_at` (carried forward from
--                            statement A by the API), keyset-paginated.
--                            Returns the 7 DB-side fields of
--                            TransactionListItem. Memo / other heavy fields
--                            are NOT carried by list rows — they belong to
--                            the transaction detail endpoint's E3 heavy block
--                            (ADR 0029) and are not fetched here.
-- Inputs:
--   $1  :sequence    BIGINT      the ledger sequence to fetch
--   $2  :closed_at   TIMESTAMPTZ ledger's closed_at, carried from statement A
--                                (used as the partition predicate; equality,
--                                NOT a range — every transaction in a ledger
--                                shares the ledger's exact closed_at)
--   $3  :cursor_ts   TIMESTAMPTZ optional keyset cursor `created_at` (NULL → start)
--   $4  :cursor_id   BIGINT      optional keyset cursor `id`           (NULL → start)
--   $5  :limit       INT         page size (caller-side clamped, default 20, max 100)
-- Indexes:      ledgers PK (sequence) — main lookup AND prev/next LATERAL
--                                       (PK is index-only-scan friendly,
--                                       avoids heap fetch needed by the
--                                       secondary idx_ledgers_closed_at);
--               transactions PK / partition routing on (created_at, id) +
--               idx_tx_ledger (ledger_sequence) within the pruned partition.
-- Notes:
--   • SUPERSESSION NOTE (2026-04): the original 0167 framing of this
--     endpoint called for embedded transactions[] to come from a per-ledger
--     S3 JSON blob (`s3://<bucket>/parsed_ledger_{N}.json`). That storage
--     track was abandoned by ADR 0029 — no parsed-ledger artifacts are
--     persisted on our side. The replacement is the same DB-only pattern
--     used by `GET /v1/transactions` list (see `crates/api/src/transactions/handlers.rs`):
--     project the structural fields from the DB `transactions` table (hash,
--     ledger_sequence, source_account, successful, fee_charged, created_at,
--     operation_count) and stop there. Memo and other heavy fields belong
--     to the transaction detail endpoint's E3 heavy block, not to list rows.
--   • This endpoint DOES query the `transactions` partition (with full
--     partition pruning via `created_at = $closed_at` from statement A).
--     The 0167 "exception case" framing — "embedded list lives off-DB" —
--     no longer applies once ADR 0029 lands.
--   • prev/next are computed via LATERAL with LIMIT 1 — one index-scan
--     seek each on the `ledgers` PK (sequence). PK was chosen over the
--     secondary idx_ledgers_closed_at because (a) projecting just
--     `sequence` from PK enables an index-only scan (no heap fetch),
--     and (b) it leans on Stellar's monotonic sequence ↔ closed_at
--     relation: the previous/next indexed sequence is the previous/next
--     closed ledger, so the answer is identical.
--     Returning NULL at chain head/tail so the API can render "no next"
--     / "no prev" navigation controls correctly.
--   • Statement B uses the standard (created_at DESC, id DESC) keyset
--     pagination from task 0043 (`TsIdCursor`), with the `(created_at, id)`
--     predicate inlined here — no shared `push_*` helper in
--     `common/pagination.rs`; each resource owns its own predicate SQL.
--     The `created_at = $closed_at` predicate fully prunes to the single
--     monthly partition that owns this ledger's transactions — no range
--     scan, no cross-partition work.

-- =====================================================================
-- Statement A — ledger header + navigation
-- =====================================================================

SELECT
    l.sequence,
    encode(l.hash, 'hex')                    AS hash_hex,
    l.closed_at,
    l.protocol_version,
    l.transaction_count,
    l.base_fee,
    prev.sequence                            AS prev_sequence,
    nxt.sequence                             AS next_sequence
FROM ledgers l
LEFT JOIN LATERAL (
    SELECT sequence
    FROM ledgers
    WHERE sequence < l.sequence
    ORDER BY sequence DESC
    LIMIT 1
) prev ON TRUE
LEFT JOIN LATERAL (
    SELECT sequence
    FROM ledgers
    WHERE sequence > l.sequence
    ORDER BY sequence ASC
    LIMIT 1
) nxt ON TRUE
WHERE l.sequence = $1;

-- @@ split @@

-- =====================================================================
-- Statement B — embedded transactions[] for this ledger
--   • Run only after statement A has resolved $1's `closed_at` (passed as $2).
--   • Project the slim TransactionListItem shape (mirrors SQL 02 list).
--   • Keyset cursor on (created_at, id) DESC via TsIdCursor (task 0043).
-- =====================================================================

SELECT
    t.id,
    encode(t.hash, 'hex')                    AS hash,
    t.ledger_sequence,
    a.account_id                             AS source_account,
    t.successful,
    t.fee_charged,
    t.created_at,
    t.operation_count
FROM transactions t
JOIN accounts a ON a.id = t.source_id
WHERE t.ledger_sequence = $1
  AND t.created_at      = $2                 -- partition prune (full equality)
  AND ($3::timestamptz IS NULL OR (t.created_at, t.id) < ($3, $4))
ORDER BY t.created_at DESC, t.id DESC
LIMIT $5;
