---
id: '0512'
title: 'FEATURE: classifier 80/20 — monitored-UNKNOWN + launchpad-NFT discriminator (drain the pending residual)'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0309', '0294', '0308', '0303']
tags:
  [
    parser,
    classifier,
    nft,
    completeness,
    sep-48,
    layer-data,
    priority-medium,
    effort-medium,
  ]
links: []
history:
  - date: 2026-08-21
    status: active
    who: karolkow
    note: >
      Promoted to active. This task now holds all the executable work for the
      NFT-completeness outcome that 0392 owns: the classifier 80/20 discriminator
      *and* the parser gate added on 2026-08-21. 0392's own promote mechanism is
      sequenced behind this task, because only landing this one produces a
      contract whose verdict actually flips — until then there is nothing to
      observe promoting.
      Entry state, measured against prod the same day: 66 contracts carry a
      decisive `Nft` verdict and hold zero rows anywhere (692 events lost with no
      trace); 67 contracts sit quarantined as `Other` across 18 WASM templates,
      of which the top four cover 48 (72%). Read 0308's recovered event-shape
      catalog before designing the discriminator.
  - date: 2026-06-23
    status: backlog
    who: karolkow
    note: >
      Spawned from the 0294 deep-dive on nfts_pending redundancy. The pending
      queue is NOT redundant — it is load-bearing ONLY because the WASM
      classifier is name-only and returns `Other` for bespoke NFTs. Measured
      residual is tiny (~65 contracts = ~7 WASM templates). This task is the
      actionable 80/20 increment of the strategic 0309 rebuild; the full L0-L4
      total-function (option A) stays in 0309.
---

# FEATURE: classifier 80/20 — monitored-UNKNOWN + launchpad-NFT discriminator

## Summary

`classify_contract_from_wasm_spec` (`crates/xdr-parser/src/classification.rs:101-120`)
is a name-only matcher over 5 NFT names. Bespoke NFTs whose WASM exposes none of them
classify `Other` forever, so their NFT-candidate events sit in `nfts_pending` and never
drain (the table's only remaining load-bearing reason — see auto-memory
`project-nfts-pending-load-bearing-classifier`). This task closes that gap with the
minimum change, and makes the `Other` bucket **observed** instead of silent.

## Measured residual (prod `chq`, 2026-06-23)

Distinct `nfts_pending` contracts that are `Other` WITH a WASM = **65 contracts across
only ~7 WASM templates**. They are heterogeneous:

- **Launchpad-NFT family** (custom ABI, genuine NFTs): functions `get_token_info`,
  `get_all_owned`, `get_max_token_id`, `update_token_url`, `bulk_mint`, `mint_original`,
  `is_collection_frozen`, `update_collection_info`, `freeze_collection`,
  `get_collection_info`. token_id / collection semantics. **Should classify `Nft`.**
  (auto-memory `custom_abi_nft_class_missed`.)
- **Custom financial / RWA** (NOT NFTs): functions `pay_off`, `sign_off`, `redeem`,
  `loan_status`, `set_loan_contract`, `add_vc`, `vc`, `check_paid`, `check_expired`.
  Emit `transfer`/`mint` with i128 **amounts** (not token_ids) → their pending rows are
  FALSE NFT candidates.

The other ~5,215 pending contracts (no-WASM, NULL verdict) are un-deployed SACs handled
by the task-0294 detection gate — out of scope here.

## Design (option B — the 80/20)

1. **Monitored-UNKNOWN.** Make `Other` non-silent: a `%UNKNOWN` metric + a per-WASM-template
   tripwire when a contract emits NFT-candidate events yet classifies `Other`. The pending
   table IS the unknown bucket; this only adds observability so new templates surface
   instead of rotting. (Core of 0309's "never SILENTLY miss".)
2. **Launchpad → `Nft`.** Add the launchpad discriminator names to
   `classify_contract_from_wasm_spec` so the family classifies `Nft` and `nft_reclassify`
   promotes its pending → hot. Prefer a small, named discriminator set (and consider
   signature-based matching) over a fragile single name.
3. **Custom-RWA verdict — OPEN DECISION** (how to drain their false pending):
   - (a) give them a `Fungible`/`Token` verdict (route_for → Drop) — data is amounts.
   - (b) keep `Other` but route `Other` + i128-amount-shaped data → Drop (data-type
     discriminator at NFT detection, not just WASM).
   - (c) leave `Other` → Pending but **monitored** — accept a handful of custom-RWA in the
     observed unknown bucket (API never reads `*_pending`, so harmless).
   - **Lean: (c)** — don't fabricate a verdict for contracts we don't fully understand;
     observe them. Decide at implementation.

## Out of scope

- Option **A** — the full L0-L4 total-function rebuild (SEP-48 spec-driven decode + SEP-46/47
  capability + typed-shape cascade) — stays in **0309** (effort-large, deferred; near-term
  value low since SEP-48 adoption ≈ 0). This task is the increment that captures most value now.
- Removing `nfts_pending` — premature until the residual is drained AND the classifier is
  total; see the deep-dive verdict (memory `project-nfts-pending-load-bearing-classifier`).

## Acceptance Criteria

- [ ] `Other`-with-NFT-events is observable (metric + tripwire), not silent
- [ ] Launchpad-NFT family classifies `Nft`; its pending promotes on `nft_reclassify`
- [ ] Custom-RWA pending drained or explicitly accepted as monitored-unknown (decision a/b/c)
- [ ] New classifier behavior unit-tested against the ~7 residual WASM shapes
- [ ] Docs: `classification.rs` doc updated to state `Other` is monitored, not silent

## Docs updated

- N/A at spawn time — fill `docs/architecture/xdr-parsing/*` (classifier behavior) when implemented.

## Renumbered from 0317 (2026-08-21)

This task was `0317`, which collided with the archived
`0317_BUG_contracts-events-ch-memory-limit`. The archived BUG kept the number —
it carries eight inbound references from completed tasks (0300, 0318, 0319),
and rewriting settled history is worse than renumbering an unstarted task.
Archived narrative in 0294 and 0425 still says "0317" and means _this_ task;
those files were deliberately left untouched.

## Scope extension — the parser gate (2026-08-21, from 0392)

Measured under [0392](../active/0392_BUG_nft-pending-live-routing-reconcile/README.md);
full evidence lives there. This task previously covered only the _classifier_
gate — contracts stamped `Other` that should be `Nft`. It now also covers the
_parser_ gate, because they are the same defect seen twice and splitting them
leaves one half ownerless. The task 0309 nominates for this work, `0308`, does
not exist — its link dangles.

### The parser gate loses more than the classifier gate quarantines

|                                                 | Contracts | Events  |
| ----------------------------------------------- | --------- | ------- |
| decisive `Nft` verdict, rows present            | 67        | 25,110  |
| decisive `Nft` verdict, **rows nowhere at all** | **66**    | **692** |

The second row is not quarantined — it is absent. No hot row, no ownership row,
no pending row, no log line. The classifier said `Nft`; the parser could not
shape the events; nothing recorded the drop.

### Cause 1 — argument shape, not event name

`stellar contract info interface --network mainnet` against the deployed WASM:

- `CB2SIYGHFGQMKEYQUWCTF3HCWBCPFUSRGVWXOPV3LIJR7K5LRPFXZEYK` —
  `transfer(env, domain: String, from: Address, to: Address)`,
  `owner_of(domain: String)`. Token identity is a **String**, in **first**
  position; canonical is `(from, to, token_id)`. This contract emits a
  perfectly canonical `transfer` event name, so name-based matching alone would
  not have rescued it. A name-only discriminator is therefore insufficient for
  this task's goal.
- `CBT5JMDOUAU3BJF7YZR42LVODLMZSQE4LIJUJNUBKEC2VZOXIF4JFBRU` —
  `owner_of(token_id: u32)`, `transfer`, `mint_badge`, `total_supply`.
  Unambiguously an NFT; mints via `mint_badge`, emits `init` / `minted`.

### Cause 2 — signature extraction is position-0-Symbol-only

`extract_event_signature` (`crates/db-clickhouse/src/persist/stage.rs:2137`)
returns `None` unless `topics[0].type == "sym"`. Two live shapes defeat it, and
they account for the single largest slice of orphan events — **470 events across
5 contracts** with a NULL `signature`:

- **namespace-first**: `["BadgeNFT", sym:"init"]`, `["StoryNFT", sym:"init"]`,
  `["identizy_nft", "minted"]` — a String collection name occupies topic 0 and
  the event name sits at topic 1.
- **String-typed event name**: `["Mint"]` as a String rather than a Symbol —
  168 events on one contract.

Neither is malformed. Both are invisible to us by construction, and because the
signature is NULL rather than wrong, they are also invisible to any monitoring
keyed on event name.

Other orphan signatures by volume: `minted` 63, `uri_upd` 58, `mint` 46, plus
`identity_minted`, `mint_event`, `transfer_event`, `set_uri_event`,
`approval_for_transfer_event`.

### Residual re-measured — the 80/20 still holds, but wider

The quarantined population is now **67 contracts across 18 distinct WASM
templates** (this task measured ~7 templates / 65 contracts on 2026-06-23 — the
contract count barely moved, the template spread nearly tripled). Concentration
still favours the 80/20:

| WASM prefix    | Contracts |
| -------------- | --------- |
| `DEFA6F74E04B` | 18        |
| `D26B246D23E8` | 13        |
| `E84BE55DCA2C` | 11        |
| `67848B7AB5A3` | 6         |

Four templates cover **48 of 67 contracts (72%)**.

### Consequence for 0392

Every contract this task promotes `Other → Nft` turns its quarantined rows into
resolved-but-stranded rows, which nothing currently promotes. 0392 owns that
mechanism and is sequenced immediately after this task. Landing this one without
0392 replaces a silent-miss defect with a stale-data defect.

### Prior art worth recovering before implementing

Task `0308` (custom-ABI NFT parser + classifier coverage) held an exhaustive
**NFT event-shape catalog** — SEP-50 + OpenZeppelin documented shapes, the
NFT-vs-SEP-41 discriminator, four verified on-chain DATA encodings, and the
custom/undocumented tail. 0308 was abandoned by decision on 2026-06-22
(`f92ce582`, code and worktree deleted with it), but the catalog is intact in
git history — 110 lines:

```bash
git show ec160781:lore/1-tasks/active/0308_FEATURE_custom-abi-nft-parser-classifier-coverage/notes/R-nft-event-shape-catalog.md
```

Read it before designing the discriminator. Do not resurrect 0308 itself — the
abandonment was deliberate and this task carries the scope now.

## Design settled — 2026-08-21, after primary-source research

Full research note:
[`0309/notes/R-spec-driven-classification-2026-08-21.md`](../backlog/0309_RESEARCH_parser-classification-design/notes/R-spec-driven-classification-2026-08-21.md)
(801 lines, every claim sourced). What follows is the decision it produced.

### The premise this task was built on is refuted by the standard

Our code states, and ADR 0046 encodes, that NFT and fungible `transfer` events are
byte-identical and only WASM classification can separate them. SEP-48 says
otherwise, in prose aimed directly at parser authors:

> "Event parsers can use the types of parameters to distinguish one type of event
> from another in the case where events share the same prefix topics, as is the
> case in some contract interfaces, e.g. [SEP-41]."

The same section carries a second instruction we also violate:

> "When matching, parsers should tolerate static topics being of the `SCVal` type
> `SCV_SYMBOL` or `SCV_STRING` because some contracts have emitted their topics
> as strings."

`topic_symbol_value` requires a Symbol. That is a documented conformance failure,
and it is the whole explanation for the 470 NULL-signature events.

### What we throw away

`crates/xdr-parser/src/contract.rs:148` keeps `ScSpecEntry::FunctionV0` and
discards every other variant — including `EventV0`, which is the contract's own
machine-readable decoding instructions: `prefixTopics`, each param's type and
`location` (`TOPIC_LIST` vs `DATA`), and `dataFormat` (`SINGLE_VALUE`/`VEC`/`MAP`).
No dependency change is needed; we already build against `stellar-xdr` 27.0.0,
which has `ScSpecEntry::EventV0`.

Of the 66 contracts with a decisive `Nft` verdict and zero rows anywhere,
**18 declare full event specs** (333 entries) and 10 more declare `*Event` UDT
structs — 42% decodable today from data we already discard.

### Why the rest do not declare — a reason code, not a mystery

`#[contractevent]` landed in soroban-sdk v23.0.0 (2025-09-03), and the SDK stamps
`rssdkver` into every Wasm. Cross-tabulated over the 66:

| SDK   | contracts | with event specs                |
| ----- | --------- | ------------------------------- |
| < 23  | 30        | **0** — the macro did not exist |
| >= 23 | 35        | 18 declare, 17 did not          |

We do not parse `contractmetav0` at ingest at all, so this reason code is
available and unused.

### Decoding cascade — the design

1. **Declared event spec** (`EventV0`) — decode against it. No guessing.
2. **`*Event` UDT structs** — same idea, weaker signal.
3. **Self-describing map payloads** — decodable with no spec at all (key names).
4. **SCVal type discriminator** — measured on prod: `Nft`-verdict `transfer`
   emits `u32` 8,487 / `map` 71 / `vec` 3; `Fungible`-verdict emits `i128`
   797,376 / `map` 17,512 / `u64` 13. **No overlap on the scalar types**; `map`
   is the only ambiguous tag and is already resolved by key (`token_id` vs
   `amount`). Caveat recorded honestly: this distribution is grouped by _our own_
   verdict, so it is strong evidence, not proof.
5. **Named UNKNOWN with a reason code** — never a silent drop.

Note the `i128` nuance that reverted "Patch C" historically: the SEP-39 contract
`CDA5FGE4…` does carry an `i128` token id, but **inside a `vec`**, so the
top-level tag is `vec`, not `i128`. The old counter-example does not contradict
tier 4.

### Classification

Structural conformance to SEP-50's 11 mandatory functions, computed from the
SEP-48 spec we already store — not a five-name match whose result depends on
which `if` runs first. Measured network-wide: **85 contracts conform, and our
classifier stamps all 85 `Nft`** — plus 48 it guessed, of which **51 of 133
verdicts** come from contracts carrying _both_ an NFT-name and a fungible-name
marker and are decided purely by `if` order.

`spec_export = false` / spec-shaking was raised as a threat to this test and
**checked**: of the 48 non-conforming `Nft`-verdict contracts, exactly **one
contract, one event** emits a canonical `(3 topics, u32)` transfer. Spec
stripping is not materially affecting classification. Limit of that check: a
fully-stripped contract would be stamped `Other` and fall outside the sample.

### Standards reality, for whoever reads this later

- **SEP-50 is a dormant Draft v0.1.0** — no content change since 2025-04-08, no
  open PR. It contradicts itself on the `approve` event shape (trait doc-comment
  vs prose); OpenZeppelin implements the prose. It specifies **no `mint`**.
- **OpenZeppelin is the de facto standard**, not SEP-50. developers.stellar.org
  deleted its own NFT pages on 2026-08-04 and now links only to OZ; SDF's own
  demo app imports `stellar_non_fungible`; no NFT implementation exists under
  `github.com/stellar/*`.
- **OZ emits no `sep` meta entry** (zero `contractmeta` in the repo), so SEP-47
  discovery cannot identify an OZ NFT. The SEP-48 spec section is the only
  reliable on-chain signal.
- **SEP-47 is Draft and self-asserted** — the spec warns contracts may lie.

### Not in this task

The quarantine's shape is 0392's call, but the research changes the input: the
`graph-node` precedent is a state enum on the row **and** a detail table **and** a
permanence flag, with fail-closed reads. Its deterministic/non-deterministic
split explains 0392's F1 exactly — the quarantine holds _deterministic_ failures
being handled by a _non-deterministic_ wait-and-retry, which is why a reconcile
moves zero rows.

## Same residual, measured from the VALUE-FLOW side (2026-09-08, from 0374)

The entry-state numbers above were measured from the quarantine queue. This is
the same residual seen from the other end — `asset_transfers`, the 0540 value
index — and it is worth recording because it says how much the residual
actually MOVES, which the queue cannot.

Full-table, no sampling: **1,352,496,561 rows, 143,782 distinct `asset_id`**.
Of those distinct ids, **28 have no `assets` row at all**, together accounting
for **563 rows** — 0.019% of the assets and 0.00004% of the traffic.

`assets` only gets a row for a contract classified `Fungible`, so an orphan
here is exactly a contract this task's discriminator has not reached. The
split:

| the 28 orphans                        | count | rows | reading                           |
| ------------------------------------- | ----- | ---- | --------------------------------- |
| classified `Nft`, in the NFT registry | ~13   | ~394 | correct by design, nothing to fix |
| classified `Other`                    | ~15   | ~162 | this task's residual              |
| unknown to `soroban_contracts` too    | **0** | 0    | every one resolves to an address  |

By movement shape rather than by verdict: **15 emit ONLY non-fungible
movements, 13 emit ONLY movements carrying an amount, 0 are mixed.** Two of
those numbers are the interesting ones:

- **3 contracts emit non-fungible movements and are NOT in the NFT registry.**
  They behave like NFTs and were classified `Other` — the launchpad-template
  class this task exists to catch, seen from the traffic side. The two busiest
  carry 88 and 44 transfer rows.
- **13 contracts emit movements WITH an amount and have no `assets` row.** They
  behave like fungible tokens and were also classified `Other`.

Nothing here is invisible: all 28 resolve to a `C…` address through
`soroban_contracts`, and the value read LEFT-joins `assets` on purpose, so
their movements render with an address instead of a code rather than
disappearing. The cost of the residual is a missing name, not a missing row.

**No task filed for this** — it is this task's subject measured from a second
angle, not a new defect. Recorded here so the discriminator's acceptance can be
checked against traffic (does the orphan count fall?) and not only against the
queue depth.
