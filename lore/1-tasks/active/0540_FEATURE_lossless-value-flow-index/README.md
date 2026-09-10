---
id: '0540'
title: 'FEATURE: lossless value-flow index — per-transfer edges replacing the net-settled aggregate'
type: FEATURE
status: active
related_adr: []
related_tasks:
  [
    '0393',
    '0411',
    '0412',
    '0413',
    '0419',
    '0536',
    '0538',
    '0541',
    '0542',
    '0543',
  ]
tags:
  [
    'clickhouse',
    'indexer',
    'api',
    'frontend',
    'phase-future',
    'effort-large',
    'priority-high',
  ]
links:
  - crates/xdr-parser/src/ledger_value.rs
  - crates/db-clickhouse/schema/init.sql
history:
  - date: 2026-09-04
    status: backlog
    who: karolkow
    note: >
      Filed after removing the `net_settled` column the same day. The
      per-(transaction, asset) aggregate carried no direction and no account, so
      an inbound and an outbound transfer rendered identically on an account
      page. Replacement is per-transfer edges. Scope set by the task owner:
      design closed AND the table live in production. SUPERSEDED 2026-09-04:
      the two-phase plan (ClickHouse-local edges first, S3 attribution later) was
      dropped for a SINGLE S3 pass, after `docs/backfills.md` put a full from-S3
      re-parse at ~a day and `Sink::with_lp_amounts_only` turned out to already
      provide a targeted write that rewrites no existing table. Asset-led
      flow tracing and L5/Effects are explicitly out of scope. Planning is a
      wayfinder map at `.wayfinder/value-flow/` (local-only, gitignored).
  - date: 2026-09-05
    status: backlog
    who: karolkow
    note: >
      Design re-reviewed end to end against code, spec and production. Four
      reversals, all measured: the sort key is Stellar's official event identity
      (the `event_index` choice rested on the dead two-phase premise); the asset
      is identified by its emitter, gated against the derived SAC address (the
      topic-string-only decoder was spoofable, 0 spoofs measured on 60k
      ledgers); no reconciliation flag in the database (core-vs-core or
      liar-vs-itself proves nothing) — a test oracle instead; muxed ids and
      memos kept from the envelope (`to_muxed_id` is 98.5% memo text). Row
      count corrected 9.05 → 5.47 bn; `asset_id` is NOT NULL after all.
  - date: 2026-09-06
    status: active
    who: karolkow
    note: >
      Promoted. Every design ticket on the map is resolved (T01–T04, T06, T07,
      T09, T10; T05 waits only on the free-space check, T11 runs after rollout).
      Implementation starts at rollout step 1: parser, three row types and
      staging, the `--only` targeted write, DDL, tests, oracle, T11 check.
  - date: '2026-09-07'
    status: active
    who: karolkow
    note: >
      Rollout steps 2-4 executed: release PR 452 merged (`098bef9d`), tag
      `production-2026.09.07-1` deployed Compute + SPA in one combined window
      with the pool adapters. L0 = 64 317 019, so the backfill range is
      50 457 424 .. 64 317 019. The window ran with no pause and no downtime:
      the four columns the new writer no longer sets were given DEFAULT NULL
      first (metadata-only), which the driver accepts, so old and new writers
      were simultaneously valid and the DROPs moved to after the backfills.
      Found on the way: the deployed API read `net_settled` in the
      transaction-list aggregate, so the runbook's drop-then-deploy order
      would have 500'd every transaction list. Steps 5-7 (binary, backfill,
      gates) remain.
---

# Lossless value-flow index

## Summary

Store one row per **transfer** — `from`, `to`, `asset`, `amount` — instead of one
aggregate per (transaction, asset), so the transaction lists can say what an
account received or sent, and so no information is lost between what the chain
recorded and what we index.

## Context

Task 0393 chose `max(Σ+, Σ−)` per (transaction, asset) and 0411 put it on every
transaction list. Two separate problems retired it:

1. **No direction, no account.** The figure is a property of the transaction, so
   an account page renders a received and a sent transfer identically.
2. **It was measurably wrong for 7.6% of value-moving transactions** — not
   because of the formula, but because the reader feeding it is blind to classic
   liquidity pools.

Measurements behind both, with method:
[notes/R-events-vs-ledger-reconciliation.md](notes/R-events-vs-ledger-reconciliation.md).

## What is settled

- **Edges, not nodes.** One row per transfer (**5.47 bn** rows, CI 4.59–6.35,
  from 30 windows across the range — an earlier 9.05 bn figure came from 600
  ledgers and sat 8.9σ high), not one per (transaction, asset, account) (19.3 bn,
  task 0536). Edges reconstruct nodes; nodes do not reconstruct edges.
- **One S3 pass, no phases** (2026-09-04). Token events in ClickHouse would have
  covered 100% of value-moving transactions, so a ClickHouse-local first phase was
  possible — but it buys days at the cost of writing ~5.5 bn rows twice, and the
  S3 pass has to happen anyway. The targeted write (`Sink::with_lp_amounts_only`,
  task 0279) makes it additive: no `--reindex`, no `repair-tier1`, no existing
  table rewritten.
- **An event ordinal is part of row identity.** Identical transfers repeat inside
  a single transaction, and — proven on production — inside a single
  **operation**. Nothing coarser than the event's position can tell them apart;
  with the official key that position is `event_pos_in_op`.
- **Row identity: Stellar's official pair, `(op_index, event_pos_in_op)`**
  (2026-09-05, reversing the 2026-09-04 choice of `event_index`). The earlier
  choice rested on a premise the one-pass decision removed: with no S3 in phase
  1, the official pair would have been empty until later, so our counter had to
  carry identity. The single S3 pass fills the pair for every row from day one.
  Verified in the stellar-rpc source (`db/event.go`): the event cursor is
  `(ledger, tx, op, event)` with the event index **reset per operation**, so the
  pair is exactly what `getEvents` returns. It is defined by the XDR itself, so
  it survives any parser change; `event_index` is our container-walk order and
  would renumber silently on a re-parse if the walk ever changed, doubling the
  table. `event_index` stays as an ordinary column (it joins `soroban_events`).
  Precondition **met** (2026-09-05, research note §10): 30 archive ledgers spread
  evenly over the range, protocols 20–27, 8 782 transactions — V4 only, 12 237
  token events, none without `op_index`. The live path (self-hosted Galexie,
  Protocol 23+) can only produce V4.
- **The ledger reader gets fixed** (option A, task owner, 2026-09-04):
  `LiquidityPoolEntry` and `ClaimableBalanceEntry` are added to
  `ledger_value.rs`. Re-scoped 2026-09-05: the reader is the **witness for the
  test oracle**, not for a stored flag (see the review section). Absorbs the
  live half of [[0412]] and [[0413]].

## Storage, settled by measurement (2026-09-05)

An independent audit re-measured the proposal on **2.89 M real edges from three
epochs** plus a 5.19 M single-partition scaling test, and corrected two of this
task's own figures:

|                | Claimed here | Measured                       | Why the claim was wrong                                                                                   |
| -------------- | ------------ | ------------------------------ | --------------------------------------------------------------------------------------------------------- |
| Rows           | 9.05 bn      | **5.47 bn** (CI 4.59–6.35)     | extrapolated from 600 ledgers = 0.004% of the range; the sample sat **8.9σ** above the mean of 30 windows |
| B/row, ZSTD(3) | 6.02         | **8.94**                       | the benchmark omitted several columns                                                                     |
| Total          | 58–74 GB     | 48.9 GB (ZSTD) / 73.5 GB (LZ4) | the two errors ran opposite ways and cancelled                                                            |

Add **+10–23% for unmerged parts** — every figure above is the floor after
`OPTIMIZE FINAL`, which production never sits at.

Adopted: **ZSTD(3) on every column** (13.43 → 8.94 B/row; the schema otherwise
inherits LZ4 by default, which nobody chose), **`index_granularity = 512`**
(rows read per account page 147 456 → 14 336, for +2.3% size; the repo has two
read-quota outages in its history, 0243/0386), and **`emitter_id` dropped** —
measured 1:1 with `asset_id` bar 6 exceptions in 2.89 M rows, with no reader.

**`LowCardinality` on the id columns — measured at scale, not adopted now.** The
audit's concern was that its gain (−34.1% at 527 k rows, −28.8% at 5.19 M)
erodes with part size. Re-measured 2026-09-05 on **16.5 M real edges in one
merged part** (the local `e2e0518` window, ledgers 55 360 000–55 423 999,
protocol 22, decoded with the final DDL), plain `Int64` against
`LowCardinality` on `asset_id`/`from_id`/`to_id`, same data at three sizes:

| Rows   | plain B/row | LC B/row | LC gain | `asset_id` | `from_id` | `to_id` |
| ------ | ----------- | -------- | ------- | ---------- | --------- | ------- |
| 525 k  | 7.94        | 6.87     | −13.4%  | −58%       | −22%      | −8%     |
| 5.41 M | 7.70        | 6.87     | −10.9%  | −60%       | −13%      | −2.5%   |
| 16.5 M | 7.57        | 6.70     | −11.6%  | −60%       | −14%      | −4%     |

The gain does **not** invert — it is flat from 5 M to 16.5 M — but it is
smaller than the audit measured and almost entirely `asset_id` (24 594 distinct
values; the account columns have 0.7–0.9 M distinct and gain little). On 5.47 bn
rows that is ~4.8 GB (estimate: 7.57 → 6.70 B/row), for a type ClickHouse flags
as suspicious (`allow_suspicious_low_cardinality_types` needed for any ≤8-byte
numeric) and extra CPU on read and merge. **Decision (task owner, 2026-09-05, option A): build the table plain.** The same saving is available later, column by column, through
`ALTER TABLE … MODIFY COLUMN asset_id LowCardinality(Int64)` — a column-only
rewrite on the real table, measured on the real distribution, reversible. Condition for that later step: the ClickHouse driver validates the
row struct against `DESCRIBE` (task 0310 outage), so the exact `ALTER` is
first run against the local Docker ClickHouse with the indexer inserting, then
on production inside a deploy window. Note
also the epoch variance: this window gives 7.57 B/row plain against the audit's
8.94 across three epochs, so the honest production range is **7.6–8.9 B/row →
41–49 GB** before the +10–23% unmerged overhead.

**`transaction_memos` and the muxed columns — sized from the envelope**
(`examples/memo_mux_audit.rs`, the same 30 archive ledgers, 8 782
transactions): **6.0% of transactions carry a memo** (text 410, id 97, hash 16),
average payload 8.4 bytes. So ~250 M rows (estimate: 6.0% × 4.13 bn), roughly
**1–3 GB** after ZSTD — not the 12–20 GB first guessed from the event-side
figure, which counted transfer _events_ and over-weights exchange payments.
Muxed **destinations: 0 of 9 476** classic destinations; muxed transaction
source: 22 (0.25%); muxed operation source: 0. The two `*_muxed_id` columns
will be almost entirely NULL and cost nothing; `from_muxed_id` is the one that
ever fills.

Disproven by measurement, so nobody re-proposes them: `Delta`/`DoubleDelta`
(−0.1%), `amount` as `Int64` with an `Int128` escape (−1.3%, and production
amounts reach 99.9996% of the `Int64` range), `FixedString(8)` for ids
(byte-identical), and the fear that `Nullable` costs anything (identical to
0.002 B/row — the null map is free after ZSTD).

Two defects the audit found in the column list, now fixed: `from_id`/`to_id` are
**not only accounts** (84% `G…`, 11–16% `L…` pools, up to 5.8% `B…` claimable
balances, up to 1.2% `C…` contracts) so each needs a namespace column; and the
read **sums**, so on a version-less `ReplacingMergeTree` it must be `FINAL` or
`GROUP BY` — a duplicate would double the balance change on screen.

A third "defect" the audit reported was **not one**: it read 855 of 2 887 253
events with no asset topic and concluded `asset_id` must be nullable. That was an
artefact of the audit's SQL, which took the asset from the topic string. The
indexer's own derivation (`event_asset_surrogate`, `ids::asset_id` type 3) gives a
bespoke token the **emitting contract's** surrogate as its `asset_id` — the same
Int64 space as classic and native ids, resolvable via `soroban_contracts`. So
`asset_id` is `Int64 NOT NULL`, and `emitter_id` stays dropped.

## Review by a second pass (2026-09-05)

The whole design was re-read against the code, the spec and production before
implementation. Four decisions came out of it, all the task owner's:

1. **Asset identity is the emitter, not the topic string.** A contract can emit
   `["transfer", from, to, "USDC:GA5ZSE…"]` — the protocol validates the emitter
   (`contract_id`, set by the host) but never the content, and CAP-67 writes
   `contract: asset` for every event. Our decoder identified the asset by the
   string alone, so a foreign contract could have appeared as real USDC on a
   victim's account page (the Etherscan fake-USDT phishing pattern). Measured on
   60 000 ledgers: 25 913 (emitter, asset-string) pairs, **25 912 equal the
   asset's registered SAC**, the one exception being `native` (no issuer to join
   on). Nobody has done it yet; the gate costs nothing because
   `sac_override_from_event_topics` (task 0323) already computes
   `emitter == derive_sac(asset)` for these events. **Decision:** a labelled
   event whose emitter is not the asset's SAC never becomes a row, is counted,
   and raises an ingest error. **Where it landed (corrected 2026-09-07 by the
   deep review):** in `extract_asset_transfers`, the decoder that feeds
   `asset_transfers` — NOT in `derive_token_event`, which still feeds the
   Tier-2 presence tables (`operation_asset_appearances`,
   `transaction_participants`) ungated. The asset page therefore still lists a
   foreign contract's `"USDC:…"` event under real USDC; closing that means
   re-emitting two 10 bn-row tables and is a follow-up task, not this one.
2. **No reconciliation flag in the database.** Who authors what: for classic and
   SAC assets stellar-core writes both the event and the balance, so they agree
   by construction; for a bespoke token the contract writes both, so agreement
   proves nothing about honesty. The flag would have compared core with core or
   a liar with itself. The one real attack — SAC spoofing — is closed by (1).
   **Decision:** events-vs-ledger reconciliation becomes a **test oracle** in the
   crate's `tests/` directory (per CLAUDE.md: verification-only code lives
   there), run on a sample window after the backfill and after every parser
   change. No column, no UI state. The reader fix (T03) is the oracle's witness.
3. **Sort key is the official identity** — see "What is settled".
4. **Muxed destinations and memos are kept, from the envelope.** A muxed address
   `M…` is `G` + a 64-bit id (SEP-23: `[type][32-byte key][8-byte id][crc]`, 69
   chars); it has no ledger entry, the balance lives on `G`. CAP-67 events keep
   the topic as `G` and put the id in `data.to_muxed_id`, **merged with the
   transaction memo** in the same field (u64 for a muxed id or MEMO_ID, string
   for MEMO_TEXT, bytes for MEMO_HASH). Measured on 27.8 M token events: no
   `M…` address ever appears in a topic; `to_muxed_id` is present on 19.9%, of
   which **98.5% is memo text**. The envelope, parsed in the same pass, carries
   both unambiguously. **Decision:** `from_muxed_id` / `to_muxed_id
Nullable(UInt64)` on the edge table (per operation, ≈0 B/row), plus a
   per-transaction `transaction_memos(ledger_sequence, application_order,
memo_type, memo)` table written by the same pass — the pass is one-shot, and
   memo is otherwise reachable only by fetching a transaction's XDR. Reverses
   the parser-boundary truncation of ADR 0026 for these two tables only. Size of
   `transaction_memos` is an **estimate** (12–20 GB) until T05 measures it.

## Supersedes (decided 2026-09-06)

**0536** (per-account settled deltas — node grain; edges contain nodes at 0.28×
the rows), **0412** and **0413** (the ledger reader's pool and claimable-balance
blind spots — their live half is the reader fix above), and **0419** (the
`net_settled` rollout and S3 re-ingest — the column is gone and this task's pass
replaces it). Status flips are a docs-only commit on `develop`. **0538** stays:
its question is wider than this table.

## What is still open

Two open tickets on the map at `.wayfinder/value-flow/` — size (T05, only the
free-space check on the box outstanding) and the closing end-to-end account
reconciliation (T11, runs after rollout). Everything else is decided.
The map is local-only by convention; this task is the committed record of the
work it plans.

## Implementation Notes

### Rollout step 1 — code (2026-09-06, in progress)

Landed on the branch, all unit tests green, clippy clean:

- **Parser** (`crates/xdr-parser`): `ExtractedEvent.event_pos_in_op` (set only in
  the per-operation loop, the `event` half of the official identity);
  `envelope::muxed_id` + `InnerTxRef::source_muxed_id`;
  `ExtractedTransaction.source_muxed_id`, `ExtractedOperation.source_muxed_id` /
  `destination_muxed_id` (Payment, both path payments, AccountMerge). New module
  `asset_transfers.rs`: `token_event_amount` (scalar / map-by-key / `token_id`),
  `extract_asset_transfers` with the emitter gate
  (`sac_override_from_event_topics`) and three reject kinds. `event.rs` was over
  the size limit, so its tests moved to `event_tests.rs`.
- **Persistence** (`crates/db-clickhouse`): `AssetTransferRow`,
  `TransactionMemoRow`, `SorobanEventOpRow`; new `persist/value_flow.rs`
  (`stage.rs` is past the limit) resolving surrogates, `*_kind`, and the muxed
  ids from the envelope matched through `op_index`; `StageInputs.asset_transfers`;
  writer streams and drains the three tables (exhaustive destructures caught
  every site); `TargetedTables` + `PartitionWriter::write_only`, with
  `write_lp_amounts_only` kept as a one-line delegate.
- **Backfill runner**: `--only <table,…>` replaces `--lp-amounts-only`
  (decision 17); `Sink::with_only`, `targeted()`.
- **Live indexer**: `parse_ledger` decodes edges per transaction, raises rejects
  as an `error!` per ledger, passes them to persistence.
- **Schema**: three DDLs in `init.sql`; statement-count test 35 → 38.
- **Tests**: 13 parser unit tests (real derived SAC addresses for the gate), 9
  staging tests (muxed from envelope vs topic, memo per tx, event ops), 4 for
  `TargetedTables`, and an e2e against a real ClickHouse proving the three row
  structs round-trip `Nullable(Int128)` / `Nullable(UInt64)` /
  `LowCardinality(String)` over RowBinary — the driver-vs-`DESCRIBE` check of
  task 0310, run before any deploy.

Also landed the same day:

- **T03** — `ledger_value.rs` reads `LiquidityPoolEntry` (the `L…` pool as
  holder of both reserves) and `ClaimableBalanceEntry` (the `B…` balance as
  holder); removal of either zeroes every balance the holder had. Pool-share
  trustlines stay unread (no event counterpart, measured). Tests moved to
  `ledger_value_tests.rs`; three new.
- **T04** — `tests/value_flow_oracle.rs`, gated on `LEDGER_CACHE_DIR`, skips
  with the exact `aws s3 cp` commands when files are missing. First run found
  the Soroban fee refund inside pre-P23 `TransactionMeta` (research note §11);
  fixed by reading the operations' changes only — `meta::operation_changes` +
  `operation_balance_deltas`. Then **0 contradictions** on 33 ledgers.

Still inside step 1: the account reconciliation check ([[T11]]) — deferred to
rollout gate 7, because it reads `asset_transfers` rows and RPC ledger state
and cannot be exercised before the backfill; and the API/frontend read path
(steps 8–10).

### Deep review, seven lenses + judge (2026-09-07)

Reviewed by fresh-context agents (correctness, simplify, security, devil's
advocate, prod readiness, architect, pattern generalisation; a separate judge
re-verified every P0/P1 in code and on production). Verdict: foundation
holds — every named decision above survived; **request changes** on one root
cause with three symptoms: the inherited `parse_token_event` assumed **one
topic shape per verb**, while mainnet has three for `mint`. Fixed in this
branch:

- **`[mint, admin, to]` (SEP-41 / `soroban-token-sdk`) credited the admin.**
  Measured: 3 169 such events, 54 emitters, in ledgers 64 000 000–64 100 000
  (versus 13.27 M CAP-67-shaped mints); e.g. tx 7850558829833248568, whose
  own `deposit` event names the recipient the decoder was dropping. Now
  decoded by shape: a second address topic selects the admin shape, the
  operand is the second address, the asset (if any) follows. Same for
  `clawback` (SEP-41 shape, not yet seen on mainnet). **Consequence**: the
  fix is in the shared parser, so from the deploy on the recipient of such a
  mint also becomes a `transaction_participants` row — a gap the presence
  index had before this task; history stays as it was until a re-parse.
- **A token verb in an unknown topic shape was dropped silently** (the
  1-topic `mint`/`burn` of concentrated-liquidity position contracts, ~120
  per 100 000 ledgers) against the module's own promise. Now
  `TransferReject::UnrecognisedTopics`, counted.
- **No emitting contract** produced a row with `asset_id = hash64("")` while
  the comment said "rejected". Now `TransferReject::NoEmitter`.
- **Per-event `warn!` → `debug!`**, one `error!` per ledger with
  `RejectCounts` by cause: a wrong passphrase would otherwise have rejected
  every labelled event at hundreds of GB of log per day on the ClickHouse box
  (the 0488 shape). An alarm needs a threshold (baseline ~150 rejects per
  500 000 ledgers on production), so it is a follow-up.
- Docs: rollout step 1 no longer tags (a tag is a deploy — 0310 order);
  rollback adds `net_settled` back before redeploying the old binary; the
  backfill loop fails the run after three failed tries instead of letting
  the watermark skip the hole; `soroban_event_ops` sized (5.07 B/row
  measured, ~29 GB); `*_kind` no longer claims a resolving table for `L`/`B`;
  "re-run is a no-op" qualified to one decoder version; the read benchmark
  says where `application_order` really comes from; decision 1 above says
  where the gate actually landed.

Deferred, with reasons, to two follow-up tasks (0542 decoder shape inventory & trust policy, 0543 value-flow read prerequisites): the SAC gate in
`derive_token_event` (Tier-2 presence tables, two 10 bn-row re-emissions), the
same admin-shape bug in `nft.rs` (`nfts.current_owner_id`), a `parser_version`
column for every decoder-fed ReplacingMergeTree (13 tables), the reject
alarm with a threshold, `<invalid-utf8>` in eight other decoders, a
resolving side table for `L…`/`B…`, and the dead `net_settled` chain
(~587 LOC, pre-existing). Two more the judge put "before rollout step 6"
landed the same day (task owner, option A): a MEMO_TEXT that is not valid
UTF-8 is stored as hex under `memo_type = 'text_hex'` (never a placeholder
a real memo could equal), and `to_muxed_id` is inherited only by the
transfer that delivers the operation's own asset (`op_delivers`: Payment
`asset`, path payments `destAsset`, AccountMerge native) — a path payment
that crosses the recipient's own offer moves other assets to the same `G…`
in the same operation, and those are not the exchange deposit the
sub-account names. Also removed on the same pass: the `write_lp_amounts_only`
delegate and the `LP_AMOUNTS` sentinel with its magic `iter()` (task owner:
legacy; the e2e uses `TargetedTables::parse`).

### Post-merge: NFT ids are not amounts (2026-09-07)

Found after the merge while the task owner challenged `token_event_amount`
against the specs: SEP-41 defines a standalone amount as `i128`, SEP-50
defines an NFT's data as its token id, "an unsigned integer" of any width —
so a `u32`/`u64`/`u128`/`u256` scalar on a token verb is a token id, and the
first decoder had summed a `u128` id as a quantity (reproduced by test,
`notes/S-nft-amount-regression.md`). Fixed on `develop` (`30753600`):
unsigned scalars and valid `{token_id}` maps → `amount = NULL`, a map with
both `amount` and `token_id` → counted reject. The oracle re-run on the fix:
0 contradictions, 0 rejects on the 33 ledgers. The one shape no parser can
tell apart — an NFT whose id is an `i128` — was measured at zero exposure on
two 500 k-ledger windows and is bounded to the collection's own `asset_id`;
it does not gate the backfill (policy and decision in
`notes/T-nft-interpretation-policy.md`; implementation is 0542 step 6).

**Correction, 2026-09-09 — the exposure is not zero.** The "zero" above was
measured on two 500 k-ledger windows; the backfilled range disagrees. Counted
across every partition `asset_transfers` holds today: **27 movements in 2
collections** are an `i128` token id stored as an `amount`. Both collections
are in `nft_ownership` (23 and 4 pieces, ids 1..9 and 1..4) and the amounts
recorded against them are 1..19 and 1..4 — sequential piece numbers, not
quantities. Neither collection has a single `amount IS NULL` row, so every
movement they have is the misread one. Consequence on the account page: the
column would print `+19` as a quantity for what is one piece changing hands.
It is masked today only because these collections have no `assets` row either,
so decision 8's flag refuses the link and the cell prints plain text — the
right answer for the wrong reason. Still bounded to the collection's own
`asset_id`, still does not gate the backfill; but 0542 step 6 now has a
measured witness instead of a hypothetical.

### Storage knobs re-challenged by the task owner (2026-09-07)

Three settings looked like overkill from the outside — "if they were that
good the whole database would use them" — so each was re-measured on the
local 16.5 M-row table with the final DDL, same data, one merged part:

**Codec.** LZ4 (the default every older table inherited) 10.96 B/row;
ZSTD(1) 7.71; **ZSTD(3) 7.57**; ZSTD(9) 7.09; `Delta`/`T64` in front of
ZSTD(3) 7.49. So ZSTD(3) is −31% against the default, ZSTD(9) buys a further
6% for much slower writes (the backfill pays that CPU), and the specialised
codecs buy 1%. The rest of the schema is LZ4 because nobody chose otherwise
at creation — except the three columns the ClickHouse pilot measured
(`soroban_events.topics_xdr` / `data_xdr`, `wasm_interface_metadata`), which
already carry ZSTD(3). Kept: ZSTD(3).

**`index_granularity`.** The account-page query (25 pairs) on three copies of
the table, rows read reported by `system.query_log`:

| Granularity     | 25 adjacent pairs | 25 spread pairs (worst) | B/row | Marks in RAM at 5.47 bn (est.) |
| --------------- | ----------------- | ----------------------- | ----- | ------------------------------ |
| 8 192 (default) | 40 960            | 188 416                 | 7.46  | ~5 MB                          |
| 2 048           | 36 864            | 65 536                  | 7.57  | ~20 MB                         |
| **512**         | 25 088            | **20 480**              | 7.57  | ~80 MB                         |

Latency barely moves locally (58 vs 66 ms); the point is the read quota
(2 bn rows per server-hour): 188 k rows per page load is ~10 k loads an hour
before the quota, 20 k rows is ~100 k. Cost of 512: +1.5% size, ~80 MB of
marks. The rest of the schema sits at 8 192 because tables were created with
defaults; range-scanned tables would gain nothing, `transaction_participants`
(the same point-read shape) probably would — a separate follow-up. Kept: 512.

**`soroban_event_ops` key.** The first DDL keyed the side table like
`soroban_events` (`ledger_sequence, transaction_id, event_index`) and measured
5.07 B/row — **4.66 of them the `transaction_id`**, a random hash that does
not compress, for 0.24 bytes of payload: ~29 GB to carry ~1.4 GB of
information. Re-keyed by the transaction's position (`ledger_sequence,
application_order, event_index`, the join going through `transactions` as
`asset_transfers` does): **0.63 B/row, ~3.6 GB**. The same two columns inside
`soroban_events` would cost 0.24 B/row (~2.5 GB on 10.4 bn rows) — the
canonical home, recorded as the target shape in task 0541; the side table is
the vehicle the S3 pass can write additively and the source of the later
per-partition fold (`ALTER … UPDATE` rewrites only the mutated columns).
Doing the fold's first half now (ALTER + row struct) would add a second
struct change to a deploy window that already carries `DROP COLUMN
net_settled` and three new tables; decision (task owner): the cheaper key
now, the fold as 0541.

**No `transaction_id` on `asset_transfers`** (task owner, 2026-09-07,
0543 step 0): the same measurement — a random hash costs ~4.7 B/row,
≈26 GB on 5.47 bn rows — for a hop through `transactions` that the account
page makes anyway. Decided before the table exists on production, because
adding a column to 5.47 bn rows afterwards is a rewrite.

Revised disk budget (estimates; floor after merges, +10–23% unmerged):
`asset_transfers` 41–49 GB (7.57 B/row at 5.47 bn — better than the
schema's existing narrow fact tables at 9.2–12.9 B/row on LZ4, and 90% of it
is two random account hashes plus the amount), `soroban_event_ops` ~3.6 GB,
`transaction_memos` 1–3 GB: **~46–56 GB**.

### The account-page read, checked before the backfill (2026-09-06)

The table's shape is the API's contract, and a wrong key would cost a rewrite
of 5.47 bn rows — so the read was run before any production row exists, on the
local 16.5 M-row table with the final DDL. The query is the one the page will
issue: 25 `(ledger_sequence, application_order)` pairs — the page already has
them: `transaction_participants` holds `(account_id, ledger_sequence,
transaction_id)` and the account list query joins `transactions` for
`application_order` today (`accounts/queries.rs`) — then a signed per-asset
sum for the account in context, deduplicated against unmerged RMT duplicates
by grouping on the full sort key. The measurement below covers the
`asset_transfers` leg only; the `transactions` hop is the page's existing
cost, re-measured end to end in rollout step 8.

| Case                                              | Granules | Rows read | Time                                       |
| ------------------------------------------------- | -------- | --------- | ------------------------------------------ |
| 25 pairs of one busy account (adjacent ledgers)   | 4        | 1 590     | 12 ms                                      |
| 25 pairs spread over the whole range (worst case) | 36       | 18 432    | 89 ms (`GROUP BY` dedup) / 10 ms (`FINAL`) |

A prefix seek in both cases: 0.11% of the table for the worst case, and the
per-pair cost (~740 rows) depends on `index_granularity`, not on table size,
so production reads the same ~18 k rows per page against a 2 bn-row/hour
quota. `FINAL` vs `GROUP BY` dedup is a step-8 choice on real parts (one part
locally flatters `FINAL`).

### Rollout steps 2–4 executed — deployed 2026-09-07

Release PR #452 merged (`098bef9d`); tag `production-2026.09.07-1` deployed the
Compute stack and the SPA. **L₀ = 64 317 019**, the first ledger the new
indexer wrote to `asset_transfers` and therefore the backfill's `END`; the
range left to backfill is `50 457 424 .. 64 317 019`.

Verified from the surfaces that changed, not from a row count: the
`Net settled` column is gone from the transaction lists (checked against the
served bundle, 43 chunks, zero occurrences — the browser's own cache showed
the old page first, the CloudFront invalidation had worked); `asset_transfers`,
`soroban_event_ops`, `transaction_memos` and `pool_state_changes` all took live
rows within minutes; ingest lag 3 s; zero Lambda errors across 265 indexer and
21 API invocations; and no ledger carrying Soroban events since L₀ is missing
its `soroban_event_ops` rows, so the deploy boundary has no hole.

**Gate 7a passed on the live tail before the backfill started** (2026-09-07).
The coverage query the gate runs on the full range was run against the ledgers
the new indexer had already written, from L₀ onward: **1 546 984 token events
in `soroban_events` against 1 546 984 distinct edges in `asset_transfers`** —
exact, zero rejects and zero drops on 1.5 M real mainnet events. Both sides
counted by their own key (`(transaction_id, event_index)` there, the official
identity here) so unmerged duplicates cannot flatter either. This is the
cheapest possible proof that the decoder is total on live traffic, and it is
worth running before committing days of machine time to the historical pass —
a disagreement here would have repeated itself across 13.9 M ledgers.

**The window needed neither a pause nor a schema change** (task owner, option
B, reversing step 4 as written above). Two findings drove it:

1. **The deployed API read the column step 4b drops.** `max(oaa.net_settled)`
   sits in the transaction-list aggregate (`api/src/common/ch.rs`), joined with
   `try_join!` and propagated with `?`, so dropping `net_settled` before the
   deploy would have answered 500 on every transaction list — global, account,
   asset and home — for the length of the deploy. The runbook did not flag it
   because it reasoned about the writer only.
2. **A default is enough.** The insert validation was read in the driver
   source rather than recalled (`clickhouse-0.15.0`, `row_metadata.rs`,
   `InsertMetadata::to_row`): a struct field with no matching column always
   fails, but a **table column absent from the struct fails only when it has no
   default**. The INSERT carries an explicit column list, so a defaulted extra
   column is simply never mentioned.

So the four columns the new writer no longer sets — `net_settled` and
`liquidity_pool_snapshots.tvl` / `volume` / `fee_revenue` — were given
`DEFAULT NULL` before the deploy. ClickHouse treats a default-only
`MODIFY COLUMN` as metadata (confirmed after the fact: no mutation was
created, the newest entry in `system.mutations` still predated the change by
weeks). Old and new writers were then simultaneously valid against one schema,
so the deploy ran with the indexer live, the read path unbroken and no ordering
constraint at all. `liquidity_pools.share_token_id` already carried a default
and was never window-critical.

The five now-unwritten columns are dropped only **after** the backfills, which
keeps a rollback to the previous binary free for the whole rollout:

```sql
ALTER TABLE operation_asset_appearances DROP COLUMN net_settled;
ALTER TABLE liquidity_pool_snapshots DROP COLUMN tvl, DROP COLUMN volume, DROP COLUMN fee_revenue;
ALTER TABLE liquidity_pools DROP COLUMN share_token_id;
```

Two pre-deploy checks worth repeating on any future window. A full schema audit
compared all 35 tables in `init.sql` against `system.columns` on production —
no column missing anywhere, and the five new tables byte-identical to the
checked-in DDL. And the all-stack `cdk diff` in the tag's own run is the only
place parked drift becomes visible: it showed `CloudWatch` differing and out of
a plain tag's scope, shipped separately as
`production-2026.09.07-2-CloudWatch`; every other stack matched.

One process gap, recorded so it is not repeated: the post-merge fix
`30753600` landed directly on `develop`, where no test workflow runs, so it
reached the release PR without ever having been through CI. It was run locally
first (420 parser tests, 135 persistence tests, clippy clean) and the release
PR was its first full CI pass.

### Rollout steps 8–10 — the read half, built 2026-09-07

The `Balance change` column, end to end, against the live tail of the table
(L₀ onward; the historical backfill has not run, and the column is honest about
that rather than waiting for it).

**API.** `crates/api/src/accounts/balance_changes.rs`, hung off step 2 of the
existing account-transactions read: that step already holds
`(ledger_sequence, application_order)` for the page, which is the
`asset_transfers` sort-key prefix, so the transfer read is a seek and needs no
second driver. Two statements — the signed per-asset sum, then the asset
identity — and the second one only because the first cannot know its asset ids
in advance. The `resolve_accounts` hop for source accounts and the transfer read
now run concurrently (`tokio::try_join!`).

**The wire contract carries three states, and the cell keeps them apart:**

| `balance_changes` | Means                              | Cell           |
| ----------------- | ---------------------------------- | -------------- |
| `null`            | below the floor — **not measured** | `Not indexed`  |
| `[]`              | measured; balances held            | `0`            |
| non-empty         | measured; these assets moved       | signed amounts |

`VALUE_FLOW_FLOOR_LEDGER = 64_317_019`, a `const` rather than config: env vars
come from the CDK compute stack, so changing it needs a deploy either way and an
env read would buy nothing. It drops to the ingest floor once the backfill
passes its gate.

**Measured on production** (25-transaction page, busiest `G…` account, one
partition — the live window is ~3 300 ledgers, so the multi-partition worst case
is not reproducible until the backfill lands):

| Statement                   | Time   | Rows read | Bytes   |
| --------------------------- | ------ | --------- | ------- |
| signed per-asset sum        | 20 ms  | 13 824    | 288 KiB |
| asset identity              | 44 ms  | 268 k     | 6.7 MiB |
| asset identity, first shape | 209 ms | 2.5 M     | 78 MiB  |

The first shape joined `soroban_contracts` through `assets.contract_id`, which
made the CTE holding the scan-only `assets` leg run twice. It does not need to:
**a Soroban asset's surrogate IS its contract's** — 4 422 of 4 422 type-3 rows
on production have `id = contract_id`, and types 0/1 have no contract at all —
so the contract leg seeks the same id list whether or not `assets` knows the
asset, and the scan happens once. `FINAL` on these dimensions is replaced by
`LIMIT 1 BY id` / `argMax(…, version)`; it measured 4.7× the rows read for a
collapse that is exact without it (0344's argument). `assets.id` carries no skip
index, so its leg is a scan either way; `soroban_contracts.id` and `accounts.id`
are bloom seeks.

**Decisions taken here, beyond what T07 settled:**

1. **An asset whose net change is zero is dropped server-side.** Under a heading
   that says `Balance change`, an asset that did not change is not one. It is
   also what makes the adversarial row readable: the six-hop arbitrage nets to
   exactly zero in eight assets and profits in one, and `+8` for the eight would
   bury the only content the row has. Measured on production — a real
   arbitrage page row carries nine assets of which eight net to zero.
2. **`to − from` per row, never a first-match branch.** With
   `from_id = to_id = account` a `multiIf` testing `to_id` first returns
   `+amount`, and the page shows an account paying itself. Pinned by a test
   against the SQL.
3. **A non-fungible movement renders as pieces (`+1 NFT`), not as a blank or a
   zero.** `amount` `NULL` has one cause and the count carries the direction, so
   the cell can say what happened without inventing an amount. The piece's own
   identity is NOT in this table — `nfts` / `nft_ownership` own that, and the
   cell does not claim otherwise. Zero such rows exist in the live window
   (0 of 1 771 703), so this is correctness for after the backfill, not for now.
4. **Assets are returned in CHAIN ORDER, not ranked** (task owner, 2026-09-07).
   The first implementation ordered by amount scaled by each asset's own
   `decimals`. The owner rejected it on the grounds that settle it: different
   assets have different decimals AND different prices, and this system holds no
   price for any of them, so the comparison is between quantities that are not
   comparable — a ranking presented as one would be a claim we cannot support.
   The statement now orders on `min((op_index, event_pos_in_op))`, the position
   of an asset's first edge in the transaction, and nothing re-sorts it
   afterwards. It is the only ordering available that is a fact rather than an
   interpretation.
5. **Fungible and non-fungible movements of one asset are SEPARATE entries**
   (review finding, 2026-09-07). `asset_id` is the emitting contract's
   surrogate, so a contract emitting both an `{amount}` and a `{token_id}`
   transfer shares an id between them — and `sum()` skips NULLs, so one row
   would carry a real `delta` AND a non-zero `nft_delta`, contradicting the
   DTO's own invariant. The cell branches on "is there an amount", so the piece
   movement would have vanished; and where the fungible legs netted out the row
   would still pass `HAVING` and print `0` for a transaction that changed an
   owner — the exact thing this column exists to stop. `is_non_fungible` is now
   part of the grouping key, which makes the invariant true by construction
   rather than by assertion. Exposure today is nil (1 non-fungible asset id in
   2.26 M rows, and it emits nothing else), which is why it survived every test.
6. **Review fixes carried in the same change** (two-axis review, 2026-09-07).
   `decode_smoke` and the `accounts/queries.rs` tests both moved to sibling
   files — the second because this change pushed that file from 798 to 831
   lines, and extracting its tests returns it to 785. The `assets.id` bloom
   index was REVERTED out of `init.sql`: it is a real optimisation but it
   creates a production migration that no rollout step owns, and the read is
   correct without it, so it belongs to its own task. A non-fungible entry now
   links to its NFT collection (`/nfts?contract=…`) instead of `/assets/…`,
   which 404s for precisely the collections this column names — they have no
   `assets` row, which is why the read joins that table `LEFT`. And
   `scaleByDecimals` now refuses a `decimals` above 39: it arrives from on-chain
   metadata as a `u32`, and `10n ** BigInt(4_000_000_000)` freezes the tab.
7. **The cell links to the PIECE, not to a filtered list** (task owner,
   2026-09-07). The first fix sent a non-fungible movement to the collection
   view, and the owner rejected the result on sight: it renders as a list with a
   56-character contract id typed into a search box, which is not where someone
   clicking an NFT wants to land. The token id is genuinely absent from
   `asset_transfers` — it belongs to the piece, not to the movement — but
   `nft_ownership` has it, keyed `(contract_id, token_id, ledger_sequence,
event_order)`, so a `contract_id`-leading seek on the page's transaction ids
   answers it: measured 18 ms / 1 509 rows, and the statement only runs when a
   page actually carries a non-fungible entry (1 row in 2 256 264 today). The id
   is used ONLY when the pieces can be proven; otherwise the entry falls back
   to the collection view rather than name a piece we would be guessing at.
   Verified end to end on production: the XLEND row reads `+1 NFT #44` and
   lands on Token 44, whose artwork states the same `$4.68K` supply as the
   `−4 681.51 USDC` beside it.

   **Every piece of a bulk move is listed and linked separately** (task owner,
   2026-09-08). Collapsing several pieces into `+3 NFT` was the first shape and
   was rejected. Two measurements shaped the fix:

   - **Pairing one edge with one piece is impossible from the data.**
     `nft_ownership.event_order` is `0` on every row of a ten-piece transfer
     (measured), so nothing there says which event moved which token. An
     earlier claim in this task — that outgoing pieces would need an
     ownership-history window — was also wrong: the table records the NEW
     owner, and the edge already carries `to_id`, which IS that owner, so one
     join names pieces in both directions.
   - **1 659 transactions on production already move more than one piece of a
     collection**, so this is a historical majority case, not a hypothetical;
     it is simply absent from the live window since L₀.

   So the read groups non-fungible movements by `(collection, transaction, new
   owner)` and expands a group into one entry per piece — but ONLY when the set
   size equals the number of pieces this account moved. That count is the proof:
   the join is on the owner, so a transaction where a third party also moved
   pieces to that same owner hands back more ids than we moved, and the entry
   then stays collapsed with no id. Verified on a real ClickHouse in all three
   shapes: a 3-piece mint → three `+1 NFT #101/#102/#103` entries; a 2-piece
   send → two `−1` entries naming the pieces via the recipient; and the same
   send with a third party's piece added to the set → collapses to `−2 NFT`
   with no id.

8. **An asset is linked only where a page can answer** (2026-09-08, found by a
   contradiction sweep after the deploy). `BalanceChange.asset` took a bespoke
   token's contract StrKey from `soroban_contracts`, which has a row for EVERY
   deployed contract — but `/assets/{id}` hydrates `(3, '', 0, surrogate)` out
   of `assets`, so a token nobody registered there answers 404. Two different
   questions read as one, and the cell drew a live-looking link to a page that
   does not exist. The identity is now emitted EMPTY for a FUNGIBLE movement
   whose asset has no `assets` row, and the cell prints the code as plain text;
   a NON-FUNGIBLE entry keeps its StrKey because its destination is the NFT
   pages, which are keyed on the contract and answer for collections `assets`
   has never heard of. Measured on production: 4 of 51 421 fungible assets in a
   historical partition, 0 of 8 643 in the live one — so nothing on screen today,
   and it would have surfaced as the backfill lowers the floor. Verified against
   production rows: the two sampled unregistered contracts report
   `resolves_on_asset_page = false`, USDC reports `true`.
9. **The dedup stays `GROUP BY`.** The step-8 choice against `FINAL` was left
   open on the grounds that one local part flatters `FINAL`; production parts
   did not change the answer, and `GROUP BY` over the full sort key is the shape
   that cannot silently stop deduplicating if a version column is ever added.

**Frontend.** `web/src/pages/accounts/BalanceChangeCell.tsx` + the column on
`AccountTransactions`. Account page only. `scaleByDecimals` rejects negative raw
amounts by its own contract, so the sign is split off in the cell and
re-attached rather than widening a formatter every other caller depends on.
7 component tests cover the three states, the sign, US grouping, the NFT count
and the unregistered-token label.

**Verified end to end against a real ClickHouse** (the repo's docker instance,
seeded to cover every branch; no production cert needed, which was the cheaper
route the task owner pointed at). The seed was a one-off: the `decode_smoke`
tests that stayed behind need no fixtures, and the branch coverage below was
re-confirmed against production rows, so nothing depends on reproducing it):

| Case                                              | Expected                           | Got                                  |
| ------------------------------------------------- | ---------------------------------- | ------------------------------------ |
| unmerged RMT duplicate of one transfer            | counted once                       | `+60 970 653 780` USDC, not doubled  |
| classic asset                                     | `CODE-ISSUER` link identity        | `USDC-GA5ZSEJY…`                     |
| registered type-3 token                           | on-chain symbol + decimals         | `DEMO`, `decimals 4`                 |
| **unregistered** NFT collection (no `assets` row) | resolved via `soroban_contracts`   | `C…`, symbol `TALKMP25`              |
| non-fungible movement                             | `amount` NULL, signed piece count  | `None`, `nft_delta −1`               |
| transfer to self                                  | cancels, dropped                   | absent                               |
| round trip netting to zero                        | dropped                            | transaction absent → `[]` → cell `0` |
| ordering                                          | chain order — first movement first | USDC before native; token before NFT |

Two `decode_smoke` tests pin the wire contract against a real server — the
`Nullable` sum into `Option<String>` and `toBool` into `bool`. Neither is
checkable in pure Rust and both are the 0324 outage class.

**A 500 that only fired on some accounts, found by browsing the running
page.** `arrayJoin([…])` types an array literal from its VALUES, so a page whose
asset ids all happen to be positive yields `Array(UInt64)` and the `id` column
decodes as `UInt64` into `i64` — `schema mismatch`, a 500 on that account and on
no other. Every test before it happened to include native, whose surrogate is
negative, so all of them passed. Fixed with `CAST([…] AS Array(Int64))` and
pinned by a `decode_smoke` case with an all-positive list — asserted through the
REAL builder, not a copy of its SQL, and confirmed by removing the `CAST` and
watching only that test go red while its all-negative twin stayed green. Found on
`GBO56XB4…`, whose only asset is `XTAR` (id 8 106 068 169 672 383 637) — which
is the argument for standing the thing up against production rows rather than
trusting a green suite.

**Read cost, and a schema line that removes the rest of it.** `assets.id` is
not in that table's `ORDER BY` and carried no skip index, so the identity read
scans. `init.sql` now declares `INDEX idx_assets_id` — the same bloom
`accounts` and `soroban_contracts` already have. Production needs
`ALTER TABLE assets ADD INDEX idx_assets_id id TYPE bloom_filter(0.001)
GRANULARITY 1` + `MATERIALIZE INDEX` for it to take effect (task owner's).
Not load-bearing: the read is correct either way, and it also cheapens the
`balances` join on the same page.

**Fees are NOT in this column, and cannot be yet.** Confirmed on production:
`asset_transfers` carries token movements only — a 1 XLM payment stores exactly
10 000 000 stroops beside a `fee_charged` of 100. Folding the fee in would need
to know WHO paid it, and **40.2% of transactions in the live window are
fee-bumps** (574 554 of 1 430 830), where the payer is the envelope's
`fee_source`, not `transactions.source_id` — deliberately so, since bug 0168.
No column stores `fee_source` anywhere. Attributing the fee to the inner source
would be wrong on two rows in five, so the column stays transfers-only and the
`Fee` column beside it carries the rest.

### The read floor, and what removing it is gated on (2026-09-09)

The account page ships with a hard floor: `VALUE_FLOW_FLOOR_LEDGER` in
`crates/api/src/accounts/balance_changes.rs`, currently the deploy ledger
64 317 019, pinned by a test and honoured twice in `accounts/queries.rs`.
Below it the column renders "not indexed" rather than an empty cell, because
`asset_transfers` holds no rows there and an empty cell reads as "nothing
moved".

**Lowering it is a rollout step, not a code change.** The floor may drop to
any ledger the backfill has provably covered, and finally to the ingest floor
50 457 424 once the whole range is in and gate 7a passes on it. Below the
ingest floor it stays forever — there is no data to have.

Each drop touches **two** places, not three: the constant and the test that
pins it. The frontend holds no threshold of its own — the cell renders
"not indexed" purely on a `null` from the API, so the API is the single owner
of where the floor sits. (An earlier version of this note said three; the
frontend gate does not exist.)

**Dropped once already, 2026-09-09: 64 317 019 → 64 128 000.** A fourth
backfill worker took `64 128 000 .. 64 317 019` out of order — the archive
partition boundary below the fourteen-day mark, so the alignment the loop does
anyway buys four extra hours of history for nothing. Verified before the drop
that the upper edge leaves no hole: `ledgers` is continuous above the deploy
ledger (28 161 rows, zero missing) and no ledger above it carries a token event
without its edges, so the worker's range meets the live block at exactly one
overlapping ledger.

An intermediate drop is worth taking before the full range lands. The
backfill's three workers advance from the bottom of their own ranges, so the
newest ledgers — the ones an account page is most likely to be asked about —
are the last to arrive. A separate worker over the most recent window fills
that slice out of order in hours rather than days, and the floor can move to
the start of that window as soon as it does. The overlap this creates with the
worker that will later cover the same ledgers costs nothing: the write is
idempotent under the row key, proven on a deliberate re-run.

### The recent window, filled out of order and shipped (2026-09-09)

The three workers walking up from the ingest floor reach the newest ledgers
last, and those are the ones an account page is most often asked about. A
fourth worker took `64 128 000 .. 64 317 019` — the archive partition boundary
below the fourteen-day mark, so the alignment the loop performs anyway bought
extra history for nothing — while the other three carried on untouched. Their
ranges overlap it, so one of them will re-do the slice later at a cost of about
4% of its remaining work; the re-run is lossless under the row key, which had
already been proven deliberately.

**Coverage proven, not assumed.** Over the window: **zero** ledgers carrying a
token event without its edges, and **84 859 840 token events against
84 859 392 distinct edges**. The difference of **448** matched the worker's own
reject counters to the unit — every one of them `unrecognised_topics`. That is
the completion gate's own arithmetic, closed exactly on a real 85 M-event
window before it runs on the full range: events minus counted rejects equals
edges, with nothing unexplained in between.

Also verified before the floor moved, so the upper edge could not hide a hole:
the `ledgers` table is continuous above the deploy ledger with none missing, and
no ledger above it carries a token event without its edges. The worker's range
meets the live block at exactly one overlapping ledger.

**Shipped by a manual deploy, not a tag** (task owner). `make -C infra
deploy-production-compute` then `deploy-production-web`, run from the `develop`
worktree — the main checkout sat on an unrelated feature branch and would have
shipped the wrong code. Two consequences worth recording. The deploy carried
the **whole read path for the first time**, not just the floor: the
balance-change API and its cell had never been on production, and the assets
list's holder ordering rode along. And production now runs code ahead of
`master`, which the next release has to reconcile or its diff will appear to
ship what is already live.

Verified after: the API function and the SPA bundle both updated within a
minute of each other, the account-page chunk contains the column, zero Lambda
errors across the deploy window, ingest lag in seconds, and the column's own
arithmetic reproduced against production for a bridged-in asset later deposited
into an automated market maker — two bridge mints as positive rows, two
two-sided deposits as negative pairs, and one swap that nets both ways in a
single transaction.

### Design decisions

#### From plan

All in "What is settled" and "Review by a second pass" above.

#### Emerged

1. **Edge decode lives in `xdr-parser`, not in staging.** The map put the
   oracle in `db-clickhouse/tests`; with the decode a pure parser function
   (events → edges, no ids) the oracle compares parser output to parser output
   and belongs in `crates/xdr-parser/tests` next to `net_settled_real_corpus.rs`.
   T04's location note is superseded by this.
2. **Muxed ids matched by identity, not position.** A transfer inherits the
   op's `destination_muxed_id` only if the op's `details.destination` equals the
   transfer's `to`, and the source id only if the op (or tx) source equals
   `from` — an intermediate hop of a path payment to a pool must not inherit
   the final recipient's sub-account.
3. **`TargetedTables` is a closed list.** A Tier-1 table can never be named in
   `--only`; the parse error names the four targetable tables.
4. **An `M…` in an event topic is split too** (`split_muxed`), though measured
   at 0 on mainnet: the hash stored is always the `G…`'s.
5. **`operation_balance_deltas` beside `ledger_balance_deltas`, not instead.**
   The oracle needs the operations' changes only (the pre-P23 fee refund sits in
   `tx_changes_after`); the original reader keeps its semantics for any caller
   that wants the whole meta.
6. **T11 deferred to rollout gate 7**, not written blind: it needs
   `asset_transfers` rows and RPC ledger state, neither of which exists before
   the backfill. A check that cannot run is not a check.

**Broken/modified tests:** `net_settled_real_corpus.rs` —
`path_payment_two_accounts_net_not_gross` now expects ten rows (three pools ×
two reserves joined the four account legs) and
`claimable_balance_nets_passthrough_and_hits_0413_gap` was renamed
`…_and_sees_the_cb_holder` and expects the `B…` balance's dSTARDUST. Both
intentional: the fixtures did not change, the reader stopped being blind.

## Acceptance Criteria

- [ ] Edge table exists, keyed so that identical transfers in one operation stay
      distinct rows — verified against the measured cases, not by argument
- [ ] Sort key is `(ledger_sequence, application_order, op_index,
event_pos_in_op)`, all NOT NULL; `event_index` is an ordinary column
- [x] `TransactionMeta::V4` confirmed across the ingested range — 30 archive
      ledgers spread over it, protocols 20–27, V4 only (2026-09-05, research
      note §10); the live path is Protocol 23+ Galexie, V4 by construction
- [x] A labelled token event whose emitter is not the asset's SAC never becomes
      a row: counted, raised as an ingest error (`extract_asset_transfers`,
      2026-09-06)
- [x] `asset_id` is `Int64 NOT NULL`; a bespoke token's id is its contract's
      (`value_flow.rs` via `event_asset_surrogate`, 2026-09-06)
- [x] `ledger_value.rs` reads `LiquidityPoolEntry` and `ClaimableBalanceEntry`;
      the 7.6% discrepancy is re-measured and accounted for — the oracle
      reconciles 14 408 (holder, asset) keys over 33 ledgers incl. the six-hop
      arbitrage ledger with 0 contradictions (research note §11, 2026-09-06)
- [x] Events-vs-ledger reconciliation exists as a test oracle in
      `crates/xdr-parser/tests/value_flow_oracle.rs` (moved from
      `db-clickhouse`, Emerged #1; passes 2026-09-06, research note §11): both readers on the same `TransactionMeta`,
      compared per (transaction, asset, holder) bit-exact, three states
      (`reconciled` / `contradicted` = failure / `no_witness` = counted, for
      assets the ledger reader cannot see); ledger list in the test (3 epochs ×
      10 + the named edge cases + T11's ledgers), files fetched to a cache, skip
      when absent; run on demand after the backfill and on any parser change.
      No flag column, no UI state
- [ ] `from_muxed_id` / `to_muxed_id` filled from the envelope, and
      `transaction_memos` written by the same pass; both proven against the
      archive on a sampled range
- [ ] Phase-1 backfill covers the full ingested range, with coverage **proven**
      against an independent source, not inferred from a row count
- [ ] Rollout per the sequence in the map's T08: tables created on prod
      **before** the indexer deploy (driver validates the struct against
      `DESCRIBE`); indexer deploy in its **own window** with the
      `DROP COLUMN net_settled` `ALTER`; backfill floor → deploy ledger with the
      targeted write (`--only asset_transfers,transaction_memos,soroban_event_ops`,
      generalising 0279's `--lp-amounts-only`) from the **same commit** as the
      live indexer; the column
      ships only after the three-layer completion gate passes on the full range
      (per-partition count vs `soroban_events`, archive re-decode diff, T11)
- [x] Read path benchmarked on the account endpoint before exposure
      (0243/0386 were both read-shape outages) — 20 ms / 13 824 rows for the
      transfer read and 44 ms / 268 k for the asset identity beside it, measured
      on production 2026-09-07; the first shape of the second statement was
      209 ms / 2.5 M and was fixed before it shipped (see rollout steps 8–10)
- [x] Column `Balance change` on the **account page only** — not the global
      transaction list, the ledger page or the asset page. A balance change means
      nothing without an account in context, and the global list is where the
      removed column misled most
- [x] `amount` is `Nullable`, and `NULL` has exactly ONE cause: a non-fungible
      movement (`{token_id}`), where no amount exists by nature. No status column
      (`token_event_amount`, 2026-09-06)
- [x] A payload shape the decoder does not recognise **never becomes a row** and
      is raised as an **ingest error** (`TransferReject::UnrecognisedPayload`,
      `error!` per ledger in `parse_ledger`, 2026-09-06), not drawn in the UI — it is a developer's
      problem and nobody browsing an account can act on it. Measured: all 8 such
      events today are one emitter restating its own mint, 7 of 7 accompanied by
      a conforming `{amount}` mint in the same transaction
- [ ] **End-to-end account reconciliation** (task owner, 2026-09-05): for a
      handful of accounts younger than the ingest floor, each chosen for a
      different edge case (DEX path payments with repeated identical transfers,
      classic LP deposit/withdraw, claimable balances, Soroban SAC + bespoke
      token, muxed destination if any, merge, clawback), the signed sum of their
      `asset_transfers` rows minus XLM fees equals their current per-asset
      balance read as raw XDR via RPC `getLedgerEntries` — bit-exact, every
      asset type. Runnable from `tests/`; accounts and ledger recorded here
- [x] Direction visible on the account page in production — deployed
      2026-09-08 and read off the live page, not off a row count. Account
      `GCKBNEKI…` shows all three states at once: `+1 NFT #44 XLEND` (ledger
      64 320 740, the same transaction whose other leg is −4 681 USDC), a
      MEASURED `0` on a Manage Sell Offer, and signed amounts with US
      grouping. A `Clawback` renders as an outflow (`−1 436.3560918 ICE`) —
      the one verb never exercised on live data before the deploy
- [ ] **Docs updated** — `docs/architecture/database-schema/**`,
      `indexing-pipeline/**`, `xdr-parsing/**`, `frontend/**` per ADR 0032.
      Read half (2026-09-07): `database-schema/database-schema-overview.md`
      UPDATED (the read path and its measured cost);
      `database-schema/endpoint-queries-clickhouse/07_get_accounts_transactions.sql`
      UPDATED (step 4 of the read shape); `frontend/frontend-overview.md`
      UPDATED (§6.7 the column, §6.3 why it is not on the global list);
      `indexing-pipeline/**` **N/A — the read half writes nothing**;
      `xdr-parsing/**` **N/A — no parser change; the decode shipped with the
      write half**. Write half's own entries stay open until the backfill
