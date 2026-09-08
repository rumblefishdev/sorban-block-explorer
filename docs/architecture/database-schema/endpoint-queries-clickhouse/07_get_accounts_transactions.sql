-- ============================================================================
-- ⚠️  `contract_ids` REMOVED from the API (task 0386) — the shared helper now
--     returns `operation_types` only; the `contract_ids` array below is dead.
-- ⚠️  CH 26.3 CORRECTION (task 0243) — do NOT implement the
--     `operation_types` array with the correlated scalar
--     subqueries shown below (`… WHERE oa.transaction_id = t.id`): ClickHouse
--     26.3 rejects them with `Code: 48 NOT_IMPLEMENTED`. Use the NON-correlated
--     two-step the shipped modules use — fetch the page of tx keys, then
--     aggregate per `(ledger_sequence, transaction_id) IN (…)` with
--     `GROUP BY transaction_id`. Reuse the shared Rust helper
--     `crates/api/src/common/ch.rs::fetch_tx_list_aggregates`.
-- ============================================================================
-- ⚠️  CH READ-COST NOTE (task 0243) — the live read path is a THREE-step shape,
--     not the single JOIN below. An account's transactions span MANY ledger
--     partitions (it is active over time), so the global `/transactions`
--     single-partition prune does NOT apply here:
--       1. Driver: `transaction_participants` WHERE account_id = <surrogate>
--          — `account_id` is the LEADING primary key (ORDER BY (account_id,
--          ledger_sequence, transaction_id)), so this is an account-scoped
--          SEEK, not a scan. ORDER BY (ledger_sequence, transaction_id) +
--          keyset, LIMIT.
--       2. Fetch the ≤limit transaction rows by `(ledger_sequence, id) IN
--          (keys)` — a primary-key-prefix prune per ledger that spans
--          partitions safely. Do NOT join the driver to an unpruned
--          `transactions FINAL` (that merges the whole 3.6B-row table and blew
--          the api_reader read_rows quota in the global list — CH Code: 201).
--       3. operation_types via the shared two-step aggregate; re-order rows in
--          Rust by the driver keyset order.
--       4. `balance_changes` (task 0540) — this account's signed per-asset
--          movement per transaction, from `asset_transfers`, keyed on
--          `(ledger_sequence, application_order)` which step 2 already has and
--          which is that table's sort-key PREFIX, so it is a seek. Two
--          statements: the signed sum, then the asset identity for the ids it
--          returned. Measured on production, 25-transaction page: 20 ms /
--          13 824 rows and 44 ms / 268 k rows. Rules that are NOT optional —
--          the inner GROUP BY over the FULL sort key (version-less RMT with
--          permanent duplicates), the `intDiv(ledger_sequence, 500000)`
--          partition prune, `to - from` per row so a self-transfer cancels,
--          and `CAST(… AS Array(Int64))` on any inlined id list (ClickHouse
--          types an array literal from its values, so an all-positive list
--          becomes `Array(UInt64)` and fails to decode). Transactions BELOW
--          the index floor get `null`, never an empty list: the table has no
--          rows there and an absent measurement must not render as a zero.
--          See `crates/api/src/accounts/balance_changes.rs`.
--     Cursor keys on `(ledger_sequence, transaction_id)` on CH (PG keeps
--     `(created_at, transaction_id)`); see `transactions::dto::TxListCursor`.
-- ============================================================================
-- Endpoint:     GET /accounts/:account_id/transactions
-- Purpose:      Paginated transactions involving a given account (as source
--               OR as a participant). Default ordering: newest first.
-- Source:       backend-overview.md §6.3 / frontend-overview.md §6.7
-- Schema:       ADR 0044 (CH pilot), parallel to PG ADR 0037
-- Data sources: DB-only.
-- Inputs:
--   $1  :account_strkey      String   G-form account ID (StrKey)
--   $2  :limit               Int      page size
--   $3  :cursor_ledger       Int64    NULL on first page
--   $4  :cursor_tx_id        Int64    NULL on first page
-- Indexes:      accounts ORDER BY (id) — leading CTE resolves StrKey → id.
--               transaction_participants ORDER BY (account_id, ledger_sequence,
--                 transaction_id) — natural keyset for this scan.
--               transactions ORDER BY (ledger_sequence, application_order, id)
--                 + PARTITION BY intDiv(ledger_sequence, 500000).
--               operations_appearances ORDER BY (ledger_sequence, transaction_id,
--                 id) — LATERAL-like subquery for operation_types.
-- CH Engine:    accounts, transactions, transaction_participants,
--               operations_appearances — all Replacing (FINAL required).
-- CH Pattern:   leading id resolve via subquery; partition prune on
--               transactions via intDiv on cursor's ledger; FINAL everywhere;
--               groupUniqArray for operation_types[].
-- ADR 0044 §:   §4.2/§4.3 (Replacing partitioned), §4.5 (state Replacing),
--               §5.2 (no `created_at` — partition predicate via intDiv).
-- Notes:
--   • PG's CTE `acc` resolves the StrKey via accounts.account_id UNIQUE.
--     CH has no equivalent UNIQUE constraint; we resolve via a subquery on
--     `accounts FINAL` instead. Selectivity is high; result is one Int64
--     after FINAL.
--   • Cursor tuple drops `created_at` (§5.2) — natural keyset for CH
--     transaction_participants is `(account_id, ledger_sequence,
--     transaction_id)`. We page on `(ledger_sequence, transaction_id) <
--     ($3, $4)` since account_id is pinned in WHERE.
--   • `operation_types[]` via correlated array subquery — CH does not
--     support `LEFT JOIN LATERAL ... ON TRUE` cleanly; the idiomatic CH
--     form is a SELECT subquery in the projection that returns Array(Int16):
--     `(SELECT groupUniqArray(oa.type) FROM operations_appearances oa FINAL
--       WHERE oa.transaction_id = t.id AND oa.ledger_sequence = t.ledger_sequence)`.
--     CH 26.x runs this as a per-row correlated read; on selective
--     transaction_id + partition predicate it's fast (≤1 partition + sparse
--     PK seek per row). The Int16 array is decoded to op_type_name strings
--     in the API layer.
--   • Source StrKey via JOIN accounts FINAL on source_id.

SELECT
    lower(hex(t.hash))                                                                  AS hash_hex,
    t.ledger_sequence,
    t.application_order,
    src.account_id                                                                      AS source_account,
    t.fee_charged,
    t.successful,
    t.operation_count,
    t.has_soroban,
    (
        SELECT groupUniqArray(oa.type)
        FROM operations_appearances oa FINAL
        WHERE oa.transaction_id  = t.id
          AND oa.ledger_sequence = t.ledger_sequence
          AND intDiv(oa.ledger_sequence, 500000) = intDiv(t.ledger_sequence, 500000)
    )                                                                                   AS operation_types,
    t.id                                                                                AS cursor_tx_id
FROM transaction_participants tp FINAL
JOIN transactions t FINAL
       ON t.id = tp.transaction_id
      AND t.ledger_sequence = tp.ledger_sequence
JOIN accounts src FINAL ON src.id = t.source_id
WHERE
    tp.account_id = (SELECT id FROM accounts FINAL WHERE account_id = $1 LIMIT 1)
    AND ($3 IS NULL OR (tp.ledger_sequence, tp.transaction_id) < ($3, $4))
-- Sort direction driven by `order` query param (`asc` | `desc`, default
-- `desc`). `order=asc` flips the `<` above to `>` and DESC→ASC in lock-step.
ORDER BY tp.ledger_sequence DESC, tp.transaction_id DESC
LIMIT $2;
