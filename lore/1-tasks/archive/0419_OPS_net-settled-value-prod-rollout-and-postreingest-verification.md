---
id: '0419'
title: 'OPS: net-settled value column — prod rollout (ALTER → deploy → S3 re-ingest → assets.id → bloom) + post-reingest verification'
type: OPS
status: superseded
related_adr: ['0044']
related_tasks: ['0393', '0417', '0383']
tags:
  [
    'clickhouse',
    'indexer',
    'ops',
    'deployment',
    'phase-future',
    'effort-medium',
    'priority-high',
  ]
links:
  - crates/db-clickhouse/schema/init.sql
history:
  - date: '2026-07-21'
    status: backlog
    who: karolkow
    note: >
      Spawned from 0393 (done). The value-column CODE is merged + verified on real
      mainnet data, but the prod rollout is a hand-run, ORDER-DEPENDENT DB sequence
      that must not live as prose in an archived task. Carries every operational
      step + an explicit post-S3-reingest verification (re-cross-validate historical
      net_settled against external sources, not just trust the re-run).
  - date: 2026-07-29
    status: backlog
    who: karolkow
    note: >
      **Step 1 EXECUTED in prod.** `ALTER TABLE operation_asset_appearances ADD
      COLUMN net_settled Nullable(Int128)` ran clean; `system.columns` now reports
      it as the fourth column. Metadata-only as expected — no rewrite of the
      table's 96.30 GiB / 11.25bn rows / 85 active parts. Ingestion unaffected:
      lag stayed single-digit seconds (4s before, 8s after) and the ledger tip
      advanced ~650 ledgers across the change, which also proves the
      `operation_asset_appearances` inserts still land — they share the ledger's
      write with `ledgers`, so a rejected insert would have stalled the tip.
      Done ahead of the rest of the rollout deliberately: the indexer's row struct
      declares `net_settled` (`persist/rows.rs:355`) and the CH client emits an
      explicit column list, so until this column existed ANY indexer deploy would
      have halted ingestion on `Code 16: No such column`. That coupling is now
      gone — the remaining steps no longer gate an unrelated backend release.
      Steps 2-9 untouched. Note for whoever resumes: the API-side value read was
      REMOVED in the meantime (see [[0411]]), so step 2's deploy no longer
      exposes anything; the read has to be written back before the column means
      anything to a client.
  - date: 2026-08-06
    status: backlog
    who: karolkow
    note: >
      Index step executed on prod (ADD + MATERIALIZE idx_oaa_transaction_id,
      mutation clean, ingestion tip advanced throughout) - then re-audited:
      the index has ZERO consumers today. The net-settled read it serves was
      withdrawn from the API (see common/ch.rs and [[0411]]); every live
      query on operation_asset_appearances filters by asset_id (the leading
      key). Real cost measured: 19.87 GiB (bloom 0.001 over 11.25bn non-null
      near-unique values - the 97 MiB analogy to the mostly-NULL
      idx_oa_asset_issuer_id was wrong). 0411 plans the (ledger,tx)
      companion which would supersede the bloom entirely. DECIDED same day
      (karolkow): dropped from prod AND removed from init.sql - no paying
      for speculation; 0411 chooses bloom-vs-companion and recreation is
      one ADD+MATERIALIZE pair (minutes). The rollout plan's index step is
      hereby superseded - do not re-run it from this task.
      Also executed same day: DROP idx_oa_asset_issuer_id (the 0381
      decision), verified gone; ~97 MiB reclaimed.
  - date: 2026-09-06
    status: superseded
    who: karolkow
    by: ['0540']
    note: >
      Superseded by 0540. The column this rollout was for is gone; the S3 re-parse it owned is 0540's single targeted pass (`--only asset_transfers,transaction_memos,soroban_event_ops`), sequenced in that task's rollout plan with its own completion gate.
---

# OPS: net-settled value column — prod rollout + post-reingest verification

## Summary

Task 0393 shipped the code that computes `net_settled` per (tx, asset) from the
**ledger** at ingest. Turning it on in prod is a hand-run ClickHouse sequence
where **ordering matters** — the indexer INSERTs the new column, so the column
must exist BEFORE the new indexer deploys or every `operation_asset_appearances`
insert fails `Code 16: No such column` and ingestion halts. History (classic +
Soroban) only materialises via a full **S3 re-ingest** (there is no CH-local value
backfill — `TransactionMeta` is S3-only). This task tracks the rollout to done and
**verifies the historical values after the re-ingest** against external truth.

## Context

- Parent: [[0393]] (archived, done — code + real-data cross-validation).
- Release gate for the READ path is owned by [[0417]] (the `(ledger,tx)` companion);
  this task is the WRITE/backfill rollout up to that gate.
- Engine is version-less `ReplacingMergeTree` — the column add is a plain additive
  `ALTER`, no rebuild. `init.sql` already reflects the desired schema; prod is
  migrated by hand.

## Implementation — ordered prod steps

1. ~~**Add the column (additive, before the new indexer deploys):**~~ **DONE
   2026-07-29.**
   ```sql
   ALTER TABLE operation_asset_appearances ADD COLUMN net_settled Nullable(Int128);
   ```
   Existing rows read `NULL` (= "not computed"); the read's `HAVING … IS NOT NULL`
   hides them, so no wrong value shows pre-backfill.
2. **Deploy the new indexer** — live-forward values start writing.
3. **Full S3 re-ingest** to fold value into history (classic + Soroban) by re-running
   `stage.rs` over every ledger. **Do NOT run the 0383 token-flow backfill after the
   column exists** — it is presence-only (`net_settled: NULL`) and could win the
   version-less RMT merge and blank a live-computed value.
4. **Confirm `assets.id` backfilled** for the range — the value read does
   `INNER JOIN assets ON a.id = asset_id`; un-backfilled rows have `id = 0`, join
   nothing, and those values silently vanish.
5. **Add + `MATERIALIZE` the bloom skip index:**
   ```sql
   ALTER TABLE operation_asset_appearances
     ADD INDEX idx_oaa_transaction_id transaction_id TYPE bloom_filter(0.001) GRANULARITY 1;
   ALTER TABLE operation_asset_appearances MATERIALIZE INDEX idx_oaa_transaction_id;
   ```
   Until `MATERIALIZE` finishes only new parts are granule-pruned.
6. **RELEASE GATE** — do NOT expose the value read on prod polling until [[0417]]
   (the `(ledger,tx)` companion) lands OR a mature-partition load test clears the
   scan + two `FINAL` joins. `wants_values` scopes the cost to the single global tx
   list — keep it gated there until this passes.

## Decide BEFORE the rollout: a touched-but-unmeasured asset must not vanish

Free to change now, expensive later — the column is not yet on prod, so no data
depends on the current semantics.

The read filters `HAVING max(net_settled) IS NOT NULL AND != 0`. That is right for
"nothing settled", but it also **deletes the asset from the transaction entirely**
when the value could not be computed — and the bespoke-token case is exactly where
that happens: a token that stores balances under any key other than
`Balance(Address)` produces no ledger delta, so we silently show _nothing_ rather
than "this asset moved, amount unknown".

Silence is worse than an honest `unknown`: it makes the transaction look like it
never touched the asset. Decide the contract now:

- `NULL` = not computed → render as **unknown**, keep the asset row visible;
- `0` = genuinely nothing settled net → may stay hidden.

Related: the module doc in `crates/xdr-parser/src/ledger_value.rs` asserts that
**every** value flow settles as an Account / Trustline / ContractData balance
change. That is false — `ContractData` keys are contract-chosen (CAP-0046-05,
**Final**), so a token may keep balances anywhere. Correct that claim in the same
change; leaving it invites the next reader to trust the number.

## Post-S3-reingest verification (REQUIRED — do not just trust the re-run)

The re-ingest recomputes history; verify it the same adversarial way 0393 verified
the live path — historical values must match external truth, not just "the code ran".

7. **Re-run the gated corpus + E2E against prod-shaped data.** The 8-fixture corpus
   (`net_settled_real_corpus.rs`) + prod-resolver E2E already pin the reducer; after
   re-ingest, spot-check that the SAME transactions now carry the expected stored
   `net_settled` in prod CH (not just in the test harness).
8. **Cross-validate a sample of RE-INGESTED historical txs** (spanning eras: pre-Soroban
   classic, SAC-wrapped, bespoke type-3) against **two independent sources** — Horizon
   `/effects` + stellar CLI XDR decode (stellar.expert `meta` for unlimited retention)
   — 1:1 on at least the deltas, per the 0393 method. Pull tx hashes from prod, decode
   their meta, compare stored value vs recomputed.
9. **Confirm no NULL-blanking / no stale-`max` inflation:** verify (a) no presence-only
   NULL rows overwrote a computed value (0383 backfill ordering), and (b) where a value
   is known-wrong from a later reducer fix, budget `OPTIMIZE TABLE
operation_asset_appearances FINAL` over the affected partitions — `max(net_settled)`
   is a one-way ratchet, so a smaller corrected value stays invisible until old rows
   physically merge away.

## Acceptance Criteria

- [ ] Column added on prod before the new indexer deploy (no `Code 16` halt)
- [ ] New indexer deployed; live-forward values landing
- [ ] Full S3 re-ingest completed over the target range; 0383 backfill NOT re-run afterward
- [ ] `assets.id` confirmed backfilled for the range (no `id = 0` drop)
- [ ] Bloom index added + `MATERIALIZE`d
- [ ] Read path kept gated until 0417 or a mature-partition load test clears it
- [ ] Post-reingest: sample of historical txs (classic + SAC + bespoke, multiple eras)
      cross-validated 1:1 vs Horizon + stellar CLI
- [ ] Verified no NULL-blanking and stale-`max` handled (OPTIMIZE FINAL budgeted if needed)
