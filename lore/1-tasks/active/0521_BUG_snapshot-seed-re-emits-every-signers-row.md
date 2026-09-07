---
id: '0521'
title: 'BUG: the snapshot seed re-emits every signers row on every pass, so one of its four reported numbers carries no information'
type: BUG
status: active
related_adr: ['0057']
related_tasks: ['0463', '0503', '0515']
tags:
  [
    snapshot,
    backfill-runner,
    clickhouse,
    data-integrity,
    effort-small,
    priority-medium,
  ]
links: []
history:
  - date: '2026-08-27'
    status: active
    who: karolkow
    note: >
      Filed from the 0463 idempotency measurement (S1, checkpoint 64,132,415):
      `balances` corrections fell 44,834,785 → 1 on the second pass, while
      `account_entry_state` emitted 10,872,072 rows again — the full live-account
      set, unchanged.
---

# BUG: the snapshot seed re-emits every signers row on every pass

## Summary

`snapshot-seed`'s pass 4 iterates every live account in the checkpoint snapshot
and emits an `account_entry_state` row **unconditionally**, with no comparison
against what we already hold. The other three passes all compare first. The rows
are byte-identical at the same ReplacingMergeTree version, so the DATA is
unaffected — but the number the operator reads in `summary.txt` is a constant,
not a measurement.

## Context

The 0463 seed's whole value after the first run is as a **reconciliation** — the
second pass is supposed to answer "what changed, and is anything wrong?". The
S1 idempotency run measured that directly:

| bucket                    | run that wrote | second pass    |
| ------------------------- | -------------- | -------------- |
| `balances` corrections    | 44,834,785     | **1**          |
| `missing` classic         | 19,262,417     | **0**          |
| `closure` classic         | 22,205,262     | **0**          |
| asset stubs               | 97,108         | **0**          |
| **`account_entry_state`** | 10,871,929     | **10,872,072** |

Every other number collapsed to the churn it should be. That last row is the
outlier, and it is the shape this project has already fixed once: on
2026-08-24 the `divergent ours-newer` bucket was reporting 11,994 rows an
operator would read as "our parser and the network disagree", when it held no
disagreement at all. A number that cannot move tells the reader nothing, and
worse, occupies the slot where a real signal would appear.

The disk cost is real but secondary — ~10.9M rows at ~21 B/row compressed
(~229 MB) per `--execute`. The owner has deliberately NOT taken a recurring
reconciliation ([0515](0515_EPIC_checkpoint-snapshot-capability.md), "Deliberately
NOT taken"), so the "rewrites 10.9M rows per pass forever" framing from the S1
write-up overstated it. **The report defect is the reason to fix this, not the
bytes.**

## The site

`crates/backfill-runner/src/snapshot/seed.rs`, pass 4:

```rust
// Pass 4: entry state — one row per live account (signers, thresholds,
// flags), the FULL set, versioned on
// the entry's own ledger so the (future) live writer wins on any change.
for (id, e) in &state.accounts {
    if !e.live { continue; }
    let Some(d) = state.account_details.get(id) else { continue; };
    out.entry_states.push(AccountEntryStateRow { … });
}
```

Note the comment says "the (future) live writer" — pass 4 was written before
that writer existed, when the full set genuinely WAS the correction. The writer
deployed 2026-08-24; the comment (and the pass) never caught up.

This is the only unconditional pass. Pass 2 gates on `e.matched`, pass 3 gates
on `known_assets` / `known_accounts`, pass 1 runs the twelve-verdict rule.

## Implementation Plan

### Step 1: read our side's versions

A `fetch_entry_state_versions(sink) -> HashMap<i64, i64>` mirroring
`fetch_id_set`: sliced on `account_id` over the same `key_slices()`,
`GROUP BY account_id` with `max(last_updated_ledger)` to collapse the unmerged
RMT parts every production read must collapse.

`max`, not `argMax` over a tuple — only the version is needed, and pulling
`signer_keys` / `signer_weights` / `signer_types` for 10.9M accounts is arrays
over the wire for a comparison this step does not make (see Rejected).

### Step 2: gate pass 4 on it

Skip when `ours >= snapshot_entry.ledger`. Equal means the two describe the same
`AccountEntry`, so the write is a no-op; greater means the live writer is ahead,
which is the same `>= checkpoint` reasoning the balances path already applies.

### Step 3: report both halves

`account_entry_state` in `summary.txt` splits into the rows written and the rows
skipped as unchanged. The two sum to the live-account count, which is the
existing invariant the summary's arithmetic is checked against.

### Step 4: one test

`seed.rs` has **0** tests against 12 in its siblings, and it is the only module
that writes. The gate is the smallest honest thing to pin: ours-behind emits,
ours-equal skips, ours-ahead skips, ours-absent emits.

## Deliberately NOT in scope

- **No floor on the version read.** Every other input to this tool carries one
  ([0515](0515_EPIC_checkpoint-snapshot-capability.md) durable rule 4), because a
  short read manufactures rows for entities that already exist. This read fails
  the OTHER way: fewer known versions means MORE rows emitted, and RMT collapses
  the identical ones. A floor would also refuse the legitimate empty-table case
  (a first seed, or a fresh local database). The asymmetry belongs in a comment,
  so the missing floor reads as a decision rather than an oversight.
- **No content diff.** Comparing signer sets at an EQUAL version would catch a
  writer that wrote wrong content at the right ledger — a real defect class, and
  exactly [0503](0503_OPS_exhaustive-completeness-audit-against-network-state.md)'s
  checklist item ("`account_entry_state` diffed against `AccountEntry` signers on
  every run AFTER the 0463 seed"). It is a heavier read (arrays for 10.9M
  accounts) and an audit, not a load-path gate. Skipping on version loses nothing
  that is detected TODAY: a same-version row written now is already a no-op under
  RMT, or an arbitrary coin flip if the content differs.

## Acceptance Criteria

- [x] Pass 4 emits only accounts whose snapshot ledger is above what we hold
- [x] `summary.txt` reports written and skipped-as-unchanged, summing to the
      live-account count
- [x] A dry-run against production reports **~0 written** and ~10.9M unchanged —
      hit exactly (2026-09-02, checkpoint 64,237,951): **0 written,
      10,909,433 unchanged**, and the sum equals the snapshot's live-account
      count to the row
- [x] The version read is sliced and its missing floor is explained in place
- [x] Regression test for the four gate cases
- [x] **Docs updated** — `docs/backfills.md` describes the seed pass; the
      `account_entry_state` line in its summary walkthrough changes shape.
      `docs/architecture/**` — N/A, no schema or contract change.
- [x] **API types regenerated** — N/A, nothing under `crates/api/**`.

## Notes

Found by [0463](../archive/0463_FEATURE_account-detail-zero-trustlines-and-signers/README.md)'s
S1 idempotency run, which was itself filed because "a future run writes nothing"
had been argued from the verdict table and never observed. The measurement
confirmed six of seven predictions and falsified the seventh — this one. Worth
recording as the return on running a check whose result you believe you know.

## Measured 2026-09-02 — the gate's first production dry-run

Checkpoint 64,237,951, read-only (`dev_read`), 375.5 s. The falsifiable
prediction hit exactly:

| line                          | value          |
| ----------------------------- | -------------- |
| `account_entry_state` written | **0**          |
| unchanged                     | **10,909,433** |
| snapshot live accounts        | 10,909,433     |

The version read returned 10,936,067 accounts we hold — 26,634 more than the
snapshot's live set, which is the merged/deleted accounts whose entry-state
rows have no lifecycle column to retire them. Expected shape, not a defect.

The same run doubles as the week's reconciliation, and the newly-informative
lines carried real signal:

- **`divergent SAME ledger` (native) = 18,363** — the 0514 counter, up from
  17,739 / 17,798 / 17,838 / 17,840 across the four pre-seed dry-runs
  (64,106,239 → 64,115,135). Still accruing, as 0514 predicts until its writer
  fix lands.
- **20 new ghosts** (19 classic + 1 native, 5.4205503 XLM) — holdings we show
  open that the network no longer has, all post-seed, i.e. removals the live
  writer missed in the last week. Small, but the class belongs to 0503's
  audit; noted there rather than chased here.
- Everything else at zero or churn: 0 missing (both populations), 0 closures,
  0 stubs, 32,483,409 classic `agree`.

Numbering note: filed as 0519, renumbered to 0521 by a concurrent session
sweeping an id collision (0519 = the api-spa task). The branch is
`fix/0521_snapshot-seed-signers-version-gate`.
