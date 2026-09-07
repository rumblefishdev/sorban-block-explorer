---
id: '0514'
title: "BUG: native balance written lower than the chain's for the same ledger on Soroban transactions"
type: BUG
status: backlog
related_adr: ['0057']
related_tasks: ['0463', '0503']
tags:
  [
    bug,
    indexer,
    soroban,
    balances,
    data-integrity,
    priority-high,
    effort-medium,
  ]
links: []
history:
  - date: '2026-08-24'
    status: backlog
    who: karolkow
    note: >
      Spawned from 0463. Found by the snapshot-seed comparison: the
      divergent-same-ledger quarantine bucket turned out to be one-directional
      on its FULL population (17,798 of 17,798 rows — our amount lower), live
      (ledgers up to the checkpoint itself), and Soroban-only (106 of 107
      recent account/ledger pairs carry a Soroban tx, zero classic).
  - date: '2026-08-25'
    status: backlog
    who: karolkow
    note: >
      Root cause proven on chain: `TransactionResultMetaV1` carries three
      change containers and the indexer reads only `tx_apply_processing`, so
      the Soroban fee refund settled in `post_tx_apply_fee_processing` never
      reaches the balance. One (account, ledger) pair decoded end to end;
      92,122 charged, 27,083 refunded, and our stored value equals the
      container we read to the stroop.
---

# BUG: native balance lower than the chain's at the same ledger (Soroban path)

## Summary

For accounts touched by a Soroban transaction, the indexer records a native
XLM balance that is LOWER than what the chain holds for that exact ledger.
The write is otherwise correct (right account, right ledger); only the amount
is behind. The live writer keeps producing these.

## Measured facts (2026-08-24, checkpoints 64,106,495 / 64,107,135)

All figures from the `snapshot-seed` dry-run comparison plus independent RPC
decoding (task 0463 audit; method in that task's
`notes/V-chain-audit-method.md`).

- **17,798 rows** in the divergent-same-ledger bucket: our row and the
  chain's entry carry the SAME `lastModifiedLedgerSeq`, different amounts.
- **One-directional on the full population, not a sample**: 17,798 ours-lower,
  0 ours-higher, 0 equal.
- **Live, not historical**: ledgers run to the checkpoint itself; the newest
  band (>= 64.0M, roughly a week) holds 107 rows spread over 96 distinct
  ledgers — a continuous process. Extrapolated rate ~1,900 rows/week.
- **Second point measured 2026-09-02** (0521's dry-run, checkpoint
  64,237,951): the bucket holds **18,363** — +565 over 131,456 ledgers
  (~8.4 days), i.e. **~470 rows/week observed**. The ~1,900/week above was an
  extrapolation from one week's band and overshot ~4x; the defect is
  confirmed still accruing, just slower. Both directions unchecked in the new
  run's dump — re-verify one-directionality when the fix lands, not before.
- **Soroban-localised**: of the 107 recent (account, ledger) pairs, our
  `transactions` table shows a Soroban transaction from that account in that
  ledger for 106 — and zero classic-only pairs.
- Differences are small (typically 0.001–0.007 XLM) and cluster hard: 249
  distinct values across 1,000 sampled rows (9,614 stroops on 415 accounts,
  9,624 on 102, 77,268 on 73). Magnitudes are consistent with fee-refund-sized
  amounts.

## Hypothesis refuted (do not re-try)

"The parser misses a fourth meta container carrying the unused-resource-fee
refund." False: `TransactionMetaV4` (stellar-xdr 26.0.1) has exactly
`tx_changes_before` / `operations` / `tx_changes_after`, and
`extract_ledger_entry_changes` reads all three.

That lead pointed at the right stage and the wrong artifact — see below.

## Root cause — proven against the chain 2026-08-25

`TransactionResultMetaV1` carries THREE sources of ledger entry changes. The
indexer maps exactly one:

```rust
pub struct TransactionResultMetaV1 {
    pub fee_processing: LedgerEntryChanges,               // the charge, up front
    pub tx_apply_processing: TransactionMeta,             // the only one we read
    pub post_tx_apply_fee_processing: LedgerEntryChanges, // the refund lands here
}
```

`indexer/src/handler/process.rs:369-379` and `xdr-parser/src/transaction.rs:70-90`
map `p.tx_apply_processing`; neither `fee_processing` nor
`post_tx_apply_fee_processing` occurs anywhere in production code. The Soroban
unused-resource-fee refund is settled after the transaction, so the last
`AccountEntry` the parser can see is the post-charge, pre-refund balance —
lower by exactly the refund, every time. That is why the bucket is
one-directional: a missed refund can only understate.

The refund is not something the balance path must learn to read out of events.
It is a change container the path never opened.

### The pair, decoded end to end

Account `GDWKHSCMYA353CRY7B2WDN3KMUIZETJVE575XCPEALTFASDRMNIDF6VZ`, ledger
64,115,081 — the newest row of `divergent_same_ledger.tsv` from the 64,115,135
run.

| side                       | stroops     |
| -------------------------- | ----------- |
| ours                       | 229,243,527 |
| chain (`getLedgerEntries`) | 229,270,610 |
| difference                 | 27,083      |

`lastModifiedLedgerSeq` comes back as 64,115,081, equal to the ledger we
recorded, so the gate in 0463's audit method licenses an amount claim.

Exactly one transaction in that ledger touches the account: `7747cd67…`,
application order 937 of 942, status FAILED (`invoke_host_function: trapped`).
Its `TransactionMetaV4` holds two changes in `tx_changes_before`, zero
operations, and an EMPTY `tx_changes_after`. Both changes carry balance
229,243,527 and differ only in `seq_num`. We stored precisely what the
container held.

The transaction's own events carry the rest:

```
stage = before_all_txs   fee →  +92,122
stage = after_all_txs    fee →  −27,083
```

Three separately fetched figures close on the stroop:

- 92,122 − 27,083 = 65,039 = `fee_charged` in the transaction result
- 229,243,527 + 27,083 = 229,270,610 = the chain's entry
- the meta's balance is the state after the charge and before the refund

Why the population is ~17,798 and not every Soroban account: the next
transaction to touch an account carries a corrected balance in its own meta, so
the stale value survives only where such a transaction was that account's LAST
write. The ~1,900/week accrual is the arrival rate of that condition.

Decoding used the official `stellar` CLI, which compiles the same
`stellar-xdr` crate we depend on, so the codec is NOT an independent source. It
does not have to be here: the claim rests on three independently fetched
figures agreeing arithmetically, and on our stored value equalling the
container we read.

Not measured: this pair is a FAILED transaction. That the refund lands in the
same container for a successful one follows from the protocol, but it stays an
inference until one is decoded.

### Fix direction

Read `post_tx_apply_fee_processing` as a third change container, after
`tx_changes_after`, on the same rising index. The last-change-wins fold in
`extract_account_states` then produces the correct balance with no change to
the write policy. Whether `fee_processing` is needed as well (classic fees) is
a separate question this task should answer rather than assume.

## Scope

1. ~~**Root cause** in the Soroban write path~~ — done 2026-08-25, above:
   `post_tx_apply_fee_processing` is never read.
2. **Fix the writer**, with a regression test on real meta from an affected
   ledger.
3. **Repair the accumulated rows.** The mechanism is already designed and
   dry-run-verified in task 0463, then deliberately removed from the seed: a
   `--heal-same-ledger` mode that supersedes both sides with the NETWORK's
   value at the CHECKPOINT version (the checkpoint strictly outversions the
   tie; writing at the tied ledger would only add a third same-version
   candidate for the ReplacingMergeTree coin flip). Its dry run healed 17,798
   rows; 200 sampled healed values were verified equal to chain via RPC.
   Resurrect that flag HERE, after the writer fix — running it while the
   writer still produces new ties would decay silently.

## Why the seed (0463) does not cover this

The seed writes the network's value only for rows it classifies as missing,
closures, ghosts or snapshot-newer heals. Same-ledger ties are quarantined and
left untouched — correcting the symptom from the seed while the writer keeps
producing new ties would need re-running forever. Order matters: fix writer,
then heal once.

## Acceptance criteria

- [x] Root cause identified and written down, with one affected
      (account, ledger) pair decoded end to end as evidence
- [ ] Writer fixed; regression test on real meta from an affected ledger
- [ ] Accumulated rows healed to the chain's value (the 0463 heal design),
      full list kept as an artifact
- [ ] The standing tie-audit query (0503) returns zero NEW rows for a week of
      post-fix ledgers
