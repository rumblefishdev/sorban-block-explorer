---
id: '0542'
title: 'FEATURE: one definition of a token movement — shared by every decoder and reader'
type: FEATURE
status: backlog
related_adr: []
related_tasks:
  [
    '0540',
    '0541',
    '0383',
    '0323',
    '0453',
    '0503',
    '0409',
    '0424',
    '0376',
    '0392',
    '0512',
  ]
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

## Measured on production, 2026-09-08 — the `nft.rs` defect, with a witness

The row above was carried from 0540's review as a code reading. It now has a
concrete case, found because 0540's read path refused to trust it.

**Transaction `6338289864338569526`, ledger 56 162 678, operation position 196,
collection `CC23DRQPZAUP5MRMPDFGU5R4ISZRSCWCP4TIED2ZTVJLQCPCNDLSALAD`.** Two
pieces minted. The chain event (`soroban_events`, indices 6 and 7) reads:

```
[sym "mint", address GDJWENY5…ALAD, address CD4FTCAP…N3TX]
             ^ admin                ^ recipient (a CONTRACT)
```

| Table                            | Recipient recorded            | Correct? |
| -------------------------------- | ----------------------------- | -------- |
| `asset_transfers` (0540 decoder) | `CD4FTCAP…` (`to_kind = 'C'`) | yes      |
| `nft_ownership` (`nft.rs`)       | `GDJWENY5…` — the ADMIN       | **no**   |

The mechanism is one comparison. `try_parse_mint` calls
`extract_args(topics, data, n_addrs = 1)`, whose shape-A arm tests
`remaining_topics.len() >= n_addrs` and then takes `remaining_topics[..1]`. For
`[mint, to]` that is the recipient; for `[mint, admin, to]` the length test still
passes and the FIRST address wins, so the admin is stored as the owner. `>=`
where the shape needs `==` plus an address-count branch — the same rule 0540
put into `asset_transfers.rs`.

**Age:** `nft.rs` was written on 2026-04-01 (task 0026) against
`SEP-0050 pattern: topics = [Symbol("mint"), Address(to)]`, which its own doc
comment still states. Every admin-shape mint since has been attributed to the
admin — five months of `nfts.current_owner_id` and `nft_ownership.owner_id`.

**Not a rogue contract.** `[mint, admin, to]` is the `soroban-token-sdk` /
SEP-41 shape. Both shapes are standard; the decoder knew one of them.

**How it surfaced.** 0540's cell names a moved piece only when the count of ids
`nft_ownership` returns for `(collection, transaction, new owner)` equals the
number of pieces the account moved. Here `asset_transfers` says two pieces to
`CD4FTCAP…` and `nft_ownership` has none for that owner (its two rows sit under
the admin), so the counts disagree and the cell collapses to `+2 NFT` with no
ids rather than linking to pieces attributed to the wrong owner. Three
transactions on production behave this way today; the four larger multi-piece
mints (10, 8, 5, 5 pieces) agree and are named. So the read path is already a
live detector for this defect — worth keeping in mind when step 2 lands, since
those cells start naming pieces the moment `nft.rs` agrees.

## Re-scoped 2026-09-08 — one definition, not an inventory of shapes

The task began as "inventory the shapes the decoder meets and decide a trust
policy". A systematic sweep for CONTRADICTIONS — places where two parts of the
system answer the same question differently on real data — showed the shapes
are the symptom. The disease is that **nothing owns the answer**: the same
question is decided independently in four to five places, with different rules,
written months apart.

### "Is this non-fungible?" — four rules

| Where                      | Rule                                                                | Written    |
| -------------------------- | ------------------------------------------------------------------- | ---------- |
| `classification.rs:102`    | WASM exposes one of 5 function names                                | 2026-04-21 |
| `nft.rs:321`               | payload type is not `void`/`map`/`vec`/`error` — **accepts `i128`** | 2026-04-01 |
| `asset_transfers.rs:90`    | payload is an UNSIGNED scalar — **rejects `i128`**                  | 2026-09-07 |
| `balance_changes.rs` (API) | `amount IS NULL`                                                    | 2026-09-08 |

Rows two and three return OPPOSITE verdicts on the same event. Measured
exposure today: **0** movements carrying an amount from an NFT-classified
contract, across the live and one historical partition — so the contradiction
is latent, and step 6 below is where it gets settled.

`nft.rs` does not merely disagree; it has its own topic and payload parser
(`extract_args`, `topic_address_value`) and shares nothing with
`event_filters::parse_token_event`, which `asset_transfers` and the presence
tables both use.

### "Which address kind counts?" — four more

| Where                   | Rule                                                    |
| ----------------------- | ------------------------------------------------------- |
| `stage.rs:2690`         | `len <= 56 && starts_with('G')` (private helper)        |
| `stage.rs:819`          | the same rule **re-spelled inline**, next door          |
| `value_flow.rs:200`     | first character of the StrKey — **every kind accepted** |
| `api/common/path.rs:79` | exactly 56 chars + base32; `M` rejected                 |

So `asset_transfers` records `L`, `B` and `C` endpoints that the presence
tables discard — the schema documents 16–22% of endpoints as non-`G`. That is a
recorded decision, not a bug, but nobody owns it in one place, and the invocation
path (`stage.rs:1930`) already keeps `C`, which shows the omission is an accident.

### Measured, 2026-09-08 (production)

> **Correction, same day.** An earlier version of this table read
> `contract_type = 1` as `Fungible`. The enum is `Token = 0, Other = 1,
Nft = 2, Fungible = 3` (`domain/src/enums/contract_type.rs:23`), so `1` is
> **`Other`** — "the classifier recognised nothing", not "the classifier
> disagreed". The corrected reading is weaker as a contradiction and stronger
> as evidence for [[0512]]: the two sides do not contradict each other, one of
> them simply never had an opinion. Re-measured with the right values:
> non-fungible movements split **410 in 12 collections the classifier calls
> `Nft`** (agreement) against **136 in 3 it calls `Other`**, and asset-row
> coverage for contracts it calls `Fungible` is **4 423 of 4 423 — complete**.

| #   | Contradiction                                                                           | Exposure                                                                                                                                                          |
| --- | --------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | decoder says non-fungible, classifier says `Other` — it recognised nothing              | **136 movements, 3 collections**                                                                                                                                  |
| 2   | `nft.rs` credits the ADMIN, `asset_transfers` credits the recipient                     | witness above; 26 movements collapse in the read                                                                                                                  |
| 3   | NFT owner is a CONTRACT and the API resolves owners only via `accounts`                 | **339 of 1 089 owners (31%)** — all 339 resolve in `soroban_contracts`, so it is a read-side omission, not missing data (belongs to [[0376]], measured there too) |
| 4   | a `C` transfer endpoint with no `soroban_contracts` row                                 | 4 of 2 371                                                                                                                                                        |
| 5   | a token with >1 ownership event in one ledger — order undecidable                       | **88** ([[0424]])                                                                                                                                                 |
| 6   | `i128` — token id or amount                                                             | 0 today, latent                                                                                                                                                   |
| 7   | `M…` inside an event TOPIC is split by `value_flow` and dropped by `derive_token_event` | 0 today (no `M…` has ever appeared in a topic); the 56 muxed ids in a live partition all come from the envelope, which is the designed path                       |

Checked and found CONSISTENT, so nobody re-derives them: transfer recipients vs
`transaction_participants` (0 missing over 294 264 live and 232 523 historical
edges), `nfts.current_owner_id` vs its own ownership history (0 of 13 955),
native's surrogate (one convention, 0 empty-string rows), `canonical_id` vs
`asset_route_token`, and the three `decimals` paths.

Mitigated outside this task, in 0540, because it was already on a live
surface — but the mitigation is a PLASTER and is named as one here so it gets
removed rather than inherited. The cell stopped linking an asset whose
`assets` row is missing (4 of 51 421 fungible assets in a historical
partition). It did not ask WHY the row is missing.

**Why it is missing, traced 2026-09-08.** An `assets` row for a bespoke token
is created by `stage.rs:2119-2146` for contracts whose verdict is `Fungible`,
and that rule is complete: 4 423 of 4 423. The four dead links are contracts
the classifier calls **`Other`** — yet each one emits `{amount}` transfers, so
the chain has already demonstrated they are fungible tokens. The registry keys
on a **guess at the WASM's function names**; the evidence of what the contract
actually did is never consulted.

**The fundamental fix**, and it belongs to this task's "one definition":
register an asset from the EVIDENCE — a contract that moved a fungible amount
is a fungible asset — not from a name match. That rule is self-healing for
history, because `asset_transfers` carries the evidence for the whole range
once the backfill lands, and it removes 0540's plaster along with the
`resolves_on_asset_page` flag that exists only to route around the gap.
Sequenced after [[0512]], which is the same question asked of the classifier
itself.

**Re-measured across the whole backfilled range, 2026-09-09 — and the "four"
were two different defects wearing one symptom.** Every partition
`asset_transfers` holds was counted, not one: **8 contracts, 45 movements, 45
transactions**, against ~1.5 bn fungible movements — 0.0000030%, one in ~33
million. The figure is a moving target, not a constant: it read 36 an hour
earlier, because three more partitions landed while the measurement ran. Per
partition the worst is **6 of 53 668 distinct fungible assets (0.011%)**; the
live window since L₀ is **0 of 8 671**, so nothing is on screen today.

The 45 split by cause, and only one of them is this section's:

| Cause                                                                       | Movements | Contracts | Classifier said | In `nft_ownership`    |
| --------------------------------------------------------------------------- | --------- | --------- | --------------- | --------------------- |
| `i128` token id stored as an `amount` — [[0540]]'s correction, step 6 below | 27        | 2         | `Nft`           | yes (23 and 4 pieces) |
| Fungible token never registered — the registry-by-name gap above            | 18        | 6         | `Other`         | no                    |

So the "four `Other` contracts" was an undercount AND a conflation. Of the six
`Other` ones, two are unarguably fungible (single amounts of 10 000 000 000 000
and a 1.3–3.8 bn spread); the remaining four emit only the values `0` and `1`,
two movements each, which the evidence rule cannot classify on its own — they
are the case that needs step 6's event-spec evidence, not just a registry
rule. The two defects must be fixed in this order: resolving the `i128` ids
first stops step 6's collections from ever reaching the registry as fungible
candidates.

### What this task now owns

1. **One definition of a token movement**, in `domain`, used by `nft.rs`,
   `asset_transfers.rs` and `derive_token_event`. `nft.rs` stops having its own
   parser. This subsumes step 2 below rather than sitting beside it.
2. **One rule for which address kinds count**, in one place, with the current
   per-table policy expressed against it rather than re-spelled four times.
3. **The `nft.rs` admin-shape fix** (task owner, 2026-09-08: it belongs here,
   not as a separate change) — including the re-derivation of the historical
   `nfts` / `nft_ownership` rows it has been mis-attributing since 2026-04-01.
4. **[[0409]] is absorbed**: "arm-A NFT pollution" is this same disease in
   another table — a token event written with an NFT's id parsed as an amount,
   because the writer did not consult the same definition.

Related, NOT absorbed: [[0512]] (the classifier itself), [[0392]] (the
completeness umbrella above it), [[0424]] (ownership order), [[0376]] (contract
owners), [[0486]] (the collection view). Each keeps its own outcome; this task
supplies the shared vocabulary they all read.

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
