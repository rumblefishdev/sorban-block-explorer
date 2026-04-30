---
id: '0185'
title: 'BUG: `accounts.sequence_number` stays `0` for accounts that are tx sources within a ledger (sentinel coercion + unconditional overwrite)'
type: BUG
status: backlog
related_adr: ['0026', '0037']
related_tasks: ['0175', '0177', '0178', '0179']
tags:
  [
    priority-high,
    layer-persist,
    audit-driven,
    accounts,
    snapshot-correctness,
  ]
links:
  - crates/indexer/src/handler/persist/staging.rs
  - crates/xdr-parser/src/state.rs
  - crates/audit-harness/sql/11_accounts.sql
history:
  - date: '2026-04-30'
    status: backlog
    who: stkrolikiewicz
    note: >
      Surfaced by manual endpoint audit E06 (task 0175 follow-up,
      30k smoke 62016000-62046000 on develop binary post 0173 +
      0177 + 0181 + 0182 + 0183 + 0178 + 0179). Two top-tp_count
      accounts (`GDFAOY7JTTJYT5ZL73NJZYLKEWXTD6NLSFDHJY5AJ6EDX2A5BSPKIKLX`,
      `GCEETSI6ZGG3CS37YUFAUKCCJSCOILXL43JOJVZ435KBJ5NICDYY4EMP`) report
      `sequence_number = 0` despite being source of 340 / 659
      transactions in the window. Horizon shows the protocol-truth
      sequence (`207181727641226876` / similar) for both. Root cause
      isolated in `staging.rs:448-465` — the sentinel `-1` for
      "trustline-only state change" is coerced to `0` AND the
      `account_state_overrides` HashMap is unconditionally overwritten,
      so a later trustline-only state change in the same ledger clobbers
      a real `sequence_number` set by an earlier tx-source state change.
      Phase 2a `horizon-diff --table accounts` does not catch this
      because Horizon shows current state and the harness skips
      `sequence_number` to avoid normal snapshot drift; bug slips
      between the two surfaces.
---

# `accounts.sequence_number = 0` for tx-source accounts

## Summary

Indexed accounts that *were* the source of one or more transactions
within a backfilled ledger range can end up with `accounts.sequence_number = 0`
in our DB even though Stellar protocol bumps `seq_num` on every successful
(and most failed-fee-charged) tx sourced by that account. The root cause
is in the persist staging layer: the sentinel `-1` produced by the parser
for "trustline-only state change" is coerced to `0` and then unconditionally
overwrites any real sequence value previously staged for the same account
in the same ledger. Whichever `ExtractedAccountState` for a given account
happens to land last in the `Vec<ExtractedAccountState>` per ledger wins —
and if that last entry is trustline-only, the real sequence is lost.

## Reproduction

Dataset: 30k smoke (mainnet ledgers 62016000–62046000), develop binary
post `lore-0173 + lore-0177 + lore-0181 + lore-0182 + lore-0183 + lore-0178
+ lore-0179`. Phase 1 invariants and Phase 2c diffs all green; this bug
is **exclusively** surfaced by the manual E06 audit cross-checking
`sequence_number` against Horizon.

```sql
-- Two accounts with high tx-as-source count and seq=0 in DB
SELECT a.account_id, a.sequence_number, a.last_seen_ledger,
  (SELECT COUNT(*) FROM accounts a2
   JOIN transactions t ON t.source_id = a2.id
   WHERE a2.account_id = a.account_id
     AND t.created_at >= '2026-04-07'
     AND t.created_at < '2026-04-10') AS tx_as_source_30k
FROM accounts a
WHERE a.account_id IN (
  'GDFAOY7JTTJYT5ZL73NJZYLKEWXTD6NLSFDHJY5AJ6EDX2A5BSPKIKLX',
  'GCEETSI6ZGG3CS37YUFAUKCCJSCOILXL43JOJVZ435KBJ5NICDYY4EMP'
);
--                         account_id                        | sequence_number | tx_as_source_30k
-- ----------------------------------------------------------+-----------------+------------------
--  GDFAOY...IKLX                                           |               0 |              340
--  GCEETSI...4EMP                                          |               0 |              659
```

Cross-check against Horizon (proves the bug is in our DB, not external
truth):

```bash
curl -s 'https://horizon.stellar.org/accounts/GDFAOY7JTTJYT5ZL73NJZYLKEWXTD6NLSFDHJY5AJ6EDX2A5BSPKIKLX' \
  | jq -r '.sequence'
# 207181727641226876   (≠ 0)
```

Five random tx-source accounts from the same window picked independently
all have correct (non-zero) `sequence_number` (`169968151008665548`,
`237118375543701655`, etc.), so the bug is **partial** — affects
specific account-state interleavings, not all tx-source accounts.

## Root cause

The persist staging build path that maps
`Vec<ExtractedAccountState>` (one per account per tx) → `HashMap<String,
AccountStateOverride>` (one per account per ledger) at
`crates/indexer/src/handler/persist/staging.rs:448-465`:

```rust
let mut account_state_overrides: HashMap<String, AccountStateOverride> =
    HashMap::new();
for st in account_states {
    // sequence_number = -1 is the sentinel for "trustline-only change" —
    // we must not overwrite the existing seq_num with it.
    let seq = if st.sequence_number >= 0 {
        st.sequence_number
    } else {
        0                     // ← BUG #1: -1 coerced to 0, losing semantic
    };
    account_state_overrides.insert(   // ← BUG #2: unconditional overwrite
        st.account_id.clone(),
        AccountStateOverride {
            first_seen_ledger: st.first_seen_ledger.map(i64::from),
            sequence_number: seq,
            home_domain: st.home_domain.clone(),
        },
    );
}
```

The inline comment correctly states the contract — "we must not overwrite
the existing seq_num with it" — but the code does exactly that. Two
distinct mistakes compound:

1. **Sentinel coercion (-1 → 0):** the parser emits
   `sequence_number: -1` whenever a tx triggered only a trustline-balance
   change for an account (no `AccountEntry` change in that tx, so
   `accum.sequence_number` is `None` → `unwrap_or(-1)` at
   `crates/xdr-parser/src/state.rs:389`). The persist layer turns that
   `-1` into `0`, losing the "no information available" semantic that
   the `-1` carried.
2. **Unconditional `HashMap.insert`:** the loop calls `.insert()` for
   every `st`, which always overwrites prior values for the same
   `account_id`. So an earlier tx that set the real
   `sequence_number = N` is clobbered by a later tx whose state was
   trustline-only.

Combined, the resulting `account_state_overrides[account_id]
.sequence_number` is `0` whenever the **last** `ExtractedAccountState`
in the input `Vec` for a given account is a trustline-only emission —
regardless of how many earlier tx-source emissions carried a real
sequence.

The `upsert_accounts` SQL at `crates/indexer/src/handler/persist/write.rs:67-89`
faithfully writes whatever the staging layer hands it. The SQL's own
sentinel logic (`COALESCE(NULLIF(sq, -1), 0)`) is unreachable here
because the staging layer already coerced `-1` to `0` before
`bind`-ing.

## Hypothesis confirmed by data

- The two affected accounts (`GDFAOY...`, `GCEETSI...`) are extreme
  high-traffic addresses — they sit in 1.1M and 1.1M `transaction_participants`
  rows respectively. Probability that the **last** `ExtractedAccountState`
  for them in a given ledger is trustline-only is high (every ledger has
  hundreds of trustline-balance updates touching these wallets via
  payments to their issued assets).
- Random tx-source accounts from less-trafficked corners of the schema
  rarely have a follow-up trustline-only state in the same ledger, so
  their `sequence_number` is preserved correctly.
- The specific failure mode also matches sample 1's `tx.successful`
  breakdown: 337 failed + 3 successful — even the failed-fee-charged
  txs bump sequence at the protocol level, so they are not the source
  of the discrepancy.

## Fix sketch

Replace the unconditional insert with a merge that respects the `-1`
sentinel:

```rust
use std::collections::hash_map::Entry;

for st in account_states {
    let new_seq = st.sequence_number;        // may be < 0
    let new_first = st.first_seen_ledger.map(i64::from);
    let new_hd = st.home_domain.clone();

    match account_state_overrides.entry(st.account_id.clone()) {
        Entry::Vacant(e) => {
            // First state seen — preserve the sentinel by using 0 only
            // when no information ever arrives, but keep `new_seq` if
            // it is real (≥ 0) so the SQL layer's COALESCE(NULLIF(sq, -1), 0)
            // path sees a real value.
            e.insert(AccountStateOverride {
                first_seen_ledger: new_first,
                sequence_number: if new_seq >= 0 { new_seq } else { 0 },
                home_domain: new_hd,
            });
        }
        Entry::Occupied(mut e) => {
            let cur = e.get_mut();
            // Only overwrite sequence_number when the new value is real
            // (≥ 0). Trustline-only updates carry the -1 sentinel and
            // must NOT clobber a real sequence we saw earlier in this
            // ledger.
            if new_seq >= 0 {
                cur.sequence_number = new_seq;
            }
            // first_seen_ledger / home_domain merge: latest non-NULL
            // wins (existing semantics — both fields are monotonically
            // additive at the parser level).
            if let Some(f) = new_first {
                cur.first_seen_ledger = Some(f);
            }
            if new_hd.is_some() {
                cur.home_domain = new_hd;
            }
        }
    }
}
```

Alternative shape: stable-sort `account_states` by `(account_id,
sequence_number == -1)` before the insert loop, so `-1` entries land
first and real values always overwrite at the end. Less explicit but
fewer branches; pick whichever the implementor finds clearer.

## Acceptance Criteria

- [ ] `staging.rs:448-465` rewritten so that `sequence_number = -1`
      (trustline-only sentinel) does not overwrite a previously
      staged real sequence within the same ledger
- [ ] `crates/indexer/src/handler/persist/staging.rs` carries a unit
      test that constructs two `ExtractedAccountState`s for the same
      `account_id` (one with `sequence_number = N`, one with
      `sequence_number = -1`) in both orderings and asserts the
      resulting `account_state_overrides` has `sequence_number = N`
      regardless of order
- [ ] New Phase 1 invariant in `crates/audit-harness/sql/11_accounts.sql`:
      "every account that appears as `transactions.source_id` ≥ 1 time
      in the partition under audit has `accounts.sequence_number > 0`".
      The exact SQL must partition-prune the `transactions` scan so
      it remains operator-friendly on the 30k smoke and the eventual
      full backfill — likely shape:

      ```sql
      \echo '### IN — every tx-source account has sequence_number > 0'
      SELECT COUNT(*) AS violations,
             (SELECT array_agg(account_id) FROM (
                SELECT a.account_id FROM accounts a
                JOIN transactions t ON t.source_id = a.id
                WHERE a.sequence_number = 0
                  AND t.created_at >= $audit_window_start
                  AND t.created_at <  $audit_window_end
                GROUP BY a.account_id LIMIT 5
             ) s) AS sample
      FROM accounts a
      WHERE a.sequence_number = 0
        AND EXISTS (
            SELECT 1 FROM transactions t
            WHERE t.source_id = a.id
              AND t.created_at >= $audit_window_start
              AND t.created_at <  $audit_window_end
        );
      ```

      (The window parameters need wiring through `run-invariants.sh`
      env or hard-coded for the 30k smoke window for now.)
- [ ] Re-run audit harness Phase 1 on a clean-slate post-fix re-backfill
      of the same 30k smoke — new invariant returns 0 violations
- [ ] Re-run E06 manual audit on the post-fix dataset — `GDFAOY...`,
      `GCEETSI...`, and 5 random tx-source picks all show
      `sequence_number > 0` matching Horizon

## Notes

- **Operationally severe.** The frontend §6.7 account detail surfaces
  `sequence_number` directly. A user looking at a high-traffic exchange
  hot wallet would currently see `0`, which is wrong and obviously
  unprofessional. The fix is a one-loop change in staging — small
  surface, big visible impact.
- **Manual audit caught what automated harness missed.** The Phase 2a
  `horizon-diff --table accounts` deliberately skips `sequence_number`
  comparison because Horizon-vs-snapshot drift on that field is the
  norm (snapshots lag by definition). The bug therefore slipped past
  100 % of automated cross-checks and only surfaced when the manual
  runbook compared a specific tx-source picked-by-activity-profile
  against Horizon. This is the value of Filip's manual audit pattern;
  good follow-up would be to teach `horizon-diff` to flag
  `sequence_number == 0` on any DB account where Horizon's value is
  non-zero — a different signal from "snapshot drift" that's still
  meaningful.
- **Companion to the lore-0184 ADR drift cleanup:** this surfaced
  during the same E06 audit pass as that doc fix; the audit pattern
  produced both deliveries.
- **Out of scope:** any refactor of the `account_state_overrides`
  merge to support multi-ledger semantics — this fix is intra-ledger
  only. Cross-ledger sequence retention is already correct via the
  SQL-side `CASE WHEN EXCLUDED.last_seen_ledger >= accounts.last_seen_ledger
  AND EXCLUDED.sequence_number <> -1 THEN EXCLUDED.sequence_number ELSE
  accounts.sequence_number END` clause in `upsert_accounts`.
