---
id: '0542'
title: 'FEATURE: token-event decoder — one shape inventory, one trust policy, re-runnable'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0540', '0541', '0383', '0323', '0453', '0503']
tags:
  [
    'xdr-parsing',
    'clickhouse',
    'indexer',
    'phase-future',
    'effort-large',
    'priority-high',
  ]
links:
  - crates/xdr-parser/src/event_filters.rs
  - crates/xdr-parser/src/asset_transfers.rs
  - crates/xdr-parser/src/nft.rs
  - crates/db-clickhouse/src/persist/stage.rs
history:
  - date: 2026-09-07
    status: backlog
    who: karolkow
    note: >
      Spawned from 0540's deep review (seven lenses + judge). One root cause
      with several symptoms: `parse_token_event` is consumed by three write
      paths with three different trust policies, and every consumer assumed
      one topic shape per verb. 0540 fixed the two symptoms that reach
      `asset_transfers`; the rest is here.
---

# Token-event decoder — shape inventory, trust policy, re-runnability

## Summary

Thread 99 interpretation decision (2026-09-07):
[NFT interpretation policy](../active/0540_FEATURE_lossless-value-flow-index/notes/T-nft-interpretation-policy.md).
Distinguish declared amount, NFT identity and unresolved payload; use
event-specific, historical-version evidence rather than integer signedness
or today's name-only contract classification. Measured 2026-09-07 (policy
note, "Measured exposure"): zero bespoke `i128` token events from
NFT-classified contracts in two 500 k-ledger windows, and the exposure is
bounded to the collection's own `asset_id` — so it does **not** gate 0540's
backfill (task owner, thread 114 A). Implementation is step 6 below.

Post-merge 0540 correction (2026-09-07): unsigned scalar NFT IDs no longer
become fungible amounts. The reproducible exploit and limits are recorded in
[0540's regression note](../active/0540_FEATURE_lossless-value-flow-index/notes/S-nft-amount-regression.md).
**Still required before claiming NFT-safe numeric aggregation:** resolve
bespoke i128 token-ID ambiguity consistently for live and historical replay.
The existing NFT classifier is not yet part of the value-flow decision, and
current database verdicts must not silently classify pre-upgrade events.

Make the token-event decoder the single place that knows (a) which topic
shapes exist on mainnet and (b) who is allowed to label an event with an
asset — and make every decoder-fed table re-runnable after a decoder fix.
Today three write paths consume `parse_token_event` with three policies, the
decoder knows one shape per verb by fixed position, and a fix to it cannot be
re-applied to history without a full S3 pass.

## Context

0540's review found that the SEP-41 / `soroban-token-sdk` shape
`[mint, admin, to]` (3 169 events, 54 emitters, in ledgers
64 000 000–64 100 000) was decoded with the admin as the recipient, and that
a token verb in an unknown shape was dropped silently. Both are fixed for the
value-flow tables on 0540's branch. The same class remains elsewhere:

| Where                                               | What                                                                                                                                                               | Origin       |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------ |
| `stage.rs::derive_token_event` (Tier-2 presence)    | no SAC emitter gate — a foreign contract's `"USDC:…"` event still lists under real USDC on the asset page                                                          | pre-existing |
| `nft.rs:372-376` (`collect_addresses`)              | takes the admin as owner for a 3-topic NFT `mint` → `nfts.current_owner_id`                                                                                        | pre-existing |
| every decoder-fed version-less `ReplacingMergeTree` | after a decoder fix, old and new rows share a key and differ in content; nothing says which is which (13 tables incl. 0540's 3)                                    | pre-existing |
| `process.rs` per-ledger reject `error!`             | no `alarm` field, so no CloudWatch metric; baseline on production is ~150 rejects per 500 000 ledgers, so `> 0` is no threshold                                    | 0540         |
| `<invalid-utf8>` placeholder                        | 8 decoders persist the literal as data (`scval.rs:51,55` → `soroban_events.signature`; `operation.rs:432,464,588`; `contract.rs:162,174,231`; `invocation.rs:508`) | pre-existing |

Consequence of 0540's fix living in the shared parser: from that deploy on,
the recipient of an admin-shape mint is also a `transaction_participants`
row; history stays as it was until a re-parse.

## Implementation Plan

### Step 1: Shape inventory as a gate

`GROUP BY (signature, JSONLength(topics), type of last topic, type of data,
keys of a map data)` over `soroban_events`, per epoch. Every combination gets
an explicit decoder status — accepted / rejected-and-counted / impossible —
checked in as a test fixture. Re-run after each protocol upgrade. The 0540
oracle (33 ledgers) has no statistical power for shapes at 1e-4 frequency;
this has.

### Step 2: One trust policy

Move the SAC emitter gate (`emitter == derive_sac(asset)`,
`sac_override_from_event_topics`) into `derive_token_event`, so the Tier-2
presence tables get it too. Decide explicitly whether
`operation_asset_appearances` / `transaction_participants` history is
re-emitted (two ~10 bn-row tables) or the exposure is documented as a
boundary. Fix `nft.rs` with the same shape rule.

### Step 3: `parser_version` on decoder-fed tables

A `parser_version` column on every decoder-fed version-less
`ReplacingMergeTree`, so a decoder fix can be re-run on a range and the 0503
tie query has something to resolve on. Repo-wide; needs a deploy window
(driver validates the row struct against `DESCRIBE`).

### Step 4: Reject alarm with a threshold

`alarm` field on the per-ledger `error!` + CloudWatch metric filter +
`FILTER_MINTED_METRICS` entry, threshold from the measured baseline.

### Step 6: `i128` token ids — semantic resolution for live and replay

Resolve the one payload the parser cannot: a bespoke NFT whose token id is
an `i128`. Evidence in this order: SEP-48 event specs from the executing
WASM version (`ScSpecEntry` event entries — `contract.rs` keeps only
`FunctionV0` today), then an evidence-backed legacy decoder for named
implementations. Live and replay must apply the same immutable evidence to
the same event or both return unresolved; never today's verdict applied
backwards. Output: `amount = NULL` for a resolved NFT id, a counted reject
for unresolved; no new column on `asset_transfers`. Tests per the policy
note's list (same `i128` under FT / NFT / missing context, packed and batch
shapes, self-transfer, executable upgrade mid-history).

### Step 5: `<invalid-utf8>` → bytes

Same fix 0540 makes for `transaction_memos.memo`: keep the bytes (hex or a
typed column), never a literal indistinguishable from real content.

## Acceptance Criteria

- [ ] A checked-in shape inventory covers every (verb, topic shape, data
      shape) seen on production; each has an explicit decoder status
- [ ] One emitter gate, applied to every consumer of `parse_token_event`;
      decision recorded on Tier-2 history
- [ ] `nfts.current_owner_id` correct for 3-topic NFT mints, verified on the
      measured instances
- [ ] `parser_version` on the decoder-fed tables; a re-run on a range is
      provably resolvable
- [ ] Reject alarm fires above the measured baseline, not at `> 0`
- [ ] No `<invalid-utf8>` literal persisted anywhere
- [ ] `i128` token ids resolved by event-spec or legacy-decoder evidence,
      identically for live and replay; unresolved is a counted reject
- [ ] **Docs updated** — `docs/architecture/xdr-parsing/**`,
      `database-schema/**` per ADR 0032
- [ ] **API types regenerated** — N/A unless the API surface changes

## Notes

Sizes from 0540's pattern-generalisation pass (estimates): gate + nft.rs ~1
day; `parser_version` ~0.5 day + window; alarm ~0.5 day; utf-8 ~0.5 day;
inventory ~1 day.
