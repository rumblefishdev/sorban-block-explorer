---
id: '0516'
title: 'BUG: account_merge is not attributed to the account being merged'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0500', '0324', '0463']
tags:
  [
    backend,
    indexer,
    operations,
    data-correctness,
    clickhouse,
    priority-low,
    effort-unknown,
  ]
history:
  - date: '2026-08-26'
    status: backlog
    who: claude
    note: >
      Spawned from 0500 future work. Found while measuring why the old
      `deleted` derivation under-detected; 0500 removed the consumer rather
      than the cause, so nothing on the read path depends on this today.
---

# BUG: a merge operation does not name the account it merges

## Symptom

`operations_appearances` carries no row naming the merged account for its own
`account_merge`. Concretely, on
`GAEGXYY63CYV34TH6HDVZ3L4WCYX7AUTLNOPFCNBR3RCQIB3MVSKLAWP`:

- its last operation is an Account Merge at ledger **57,037,462**, which is
  also its `last_seen_ledger`;
- that ledger holds **exactly one** `type = 8` appearance;
- **none of the 664 appearances in that ledger** names the account as
  `source_id` or `destination_id`;
- the account reaches its own transaction list through
  `transaction_participants` only.

So the operation exists and the account participated, but the two are not
linked in the appearances table.

## Why it is filed and not fixed

Task 0500 needed "is this account deleted?" and was getting `false` for dead
accounts. The fix there was to stop deriving the answer from operation history
and read it from the lifecycle column on the account's native holding
(ADR 0055) — one keyed lookup, complete since the 0463 checkpoint seed, and
verified against the chain on 236 accounts with no exceptions.

That removed the only known consumer. This defect is therefore invisible today
on the account page, which is why it is priority-low: nothing renders wrongly
because of it.

## Scale — MEASURED 2026-08-26

Over ledgers **64,131,264–64,134,437** (3,174 ledgers, ~4.4 h), successful
`account_merge` only:

|                                                                       |                                    |
| --------------------------------------------------------------------- | ---------------------------------- |
| type-8 appearances                                                    | 2,264 (2,259 successful, 5 failed) |
| …with `source_id` NULL                                                | **2,175 — 96.3%**                  |
| distinct non-null `source_id` values                                  | 84                                 |
| distinct merged accounts after `coalesce(op.source_id, tx.source_id)` | 1,055                              |

So it is not a rare shape: **96% of merge operations name no account at all**,
and the 4% that do are 84 distinct sources — consistent with the operation
carrying an explicit `sourceAccount` only when the transaction submitter
differs from the account being closed.

The merged account is recoverable today by falling back to the TRANSACTION
source (`coalesce(op.source_id, tx.source_id)`), which is how the measurement
above identified all 1,055. That fallback is a read-side workaround, not the
fix: it is wrong for any transaction that merges an account other than its own
source, and nothing on the read path currently applies it.

Cross-checked while measuring: those 1,055 accounts all carry a lifecycle
closure stamp on their native holding (the handful that read `closed = 0`
probed PRESENT on chain — merged then re-created inside the window, so the
zero is correct). The defect is confined to `operations_appearances`
attribution; `balances` lifecycle is unaffected.

## What is NOT known

- **Older windows.** One 4.4-hour window was measured, during heavy churn-bot
  traffic. Whether the 96% ratio holds across quieter periods is unmeasured.
- **Cause.** The 96%-NULL / 84-distinct split points at `source_id` carrying
  ONLY the operation's explicit `sourceAccount` (absent on most merges, since
  the account usually submits its own closing transaction) rather than falling
  back to the transaction source. Not confirmed in the parser — read it before
  acting on this reading.
- **Blast radius.** Any other read that asks "which account did operation X"
  for an op with an explicit source account would inherit the same gap. Worth
  a grep over the API's operation queries before deciding this is harmless.

## Implementation

1. Measure first: over a bounded ledger window, count successful `type = 8`
   appearances whose `source_id` resolves to an account that a
   `transaction_participants` row for the same ledger also names — and the
   complement. The complement is the population.
2. Read the parser's operation-appearance emission for the explicit
   `sourceAccount` case, not just `account_merge`.
3. Decide whether the fix belongs in the writer or in the appearance model
   (a row per participating role rather than source/destination).
4. Re-measure to zero, and re-check whether anything else in the API was
   silently relying on the old shape.

## Acceptance criteria

- [x] Scale measured and recorded BEFORE any change, over a named window
      (64,131,264–64,134,437: 2,175 of 2,259 successful merges unattributed)
- [ ] Root cause identified in the writer, not guessed from the symptom
- [ ] Fixed, and the same window re-measured to zero
- [ ] Other operation types with an explicit `sourceAccount` checked for the
      same gap, and the result recorded either way
- [ ] Historical rows: state explicitly whether a re-parse is needed or the
      defect is forward-only
- [ ] **Docs updated** — or `N/A` with the reason
- [ ] **API types** — expected `N/A`; state it rather than leaving it blank
