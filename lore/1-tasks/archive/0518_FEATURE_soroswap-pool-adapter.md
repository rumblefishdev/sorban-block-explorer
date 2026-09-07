---
id: '0518'
title: 'Soroswap pool adapter — reserves from sync, volume from swap'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0516', '0517', '0374', '0008']
tags:
  [backend, clickhouse, api, liquidity-pools, priority-medium, effort-medium]
links:
  - 'https://github.com/rumblefishdev/soroban-block-explorer/issues/405'
history:
  - date: '2026-08-26'
    status: backlog
    who: karolkow
    note: >
      Second AMM adapter under 0516. HARD DEPENDENCY on 0517 — until event
      names resolve, every Soroswap event reads as NULL and nothing here is
      reachable.
  - date: '2026-09-02'
    status: active
    who: karolkow
    note: >
      Activated after 0517's rule landed (PR #443) and the pre-adapter
      probes settled every architecture seam. Ordering re-reversed to
      Soroswap-first on the per-swap re-measurement (see 0516).
---

# Soroswap pool adapter

## Summary

Index Soroswap pools — reserves, volume, positions — and union them into the
pool list under their own deployment label. Second adapter under
[0516](./0516_FEATURE_soroban-amm-coverage-umbrella.md).

## Blocked on 0517

Every Soroswap event currently has a `NULL` signature. Until
[0517](./0517_FIX_event-name-read-from-wrong-topic.md) lands there is nothing
to query. Do not start before it.

## What the store already holds

Once names resolve, 1 145 174 events become readable, including 572 576 `sync`
events. `sync` is the reserve announcement after every trade, so reserve
history needs no reconstruction — the same conclusion 0374 reached for Aquarius
by a different route.

## The four seams for this protocol

| Seam           | Soroswap                                                                                                                                   |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| discovery      | **no router registry** — pairs self-announce. Unlike Aquarius, this cannot be a registry read; settle factory-vs-shape with a measurement. |
| event name     | `topics[1]`, behind the `SoroswapPair` label                                                                                               |
| reserves       | `sync`                                                                                                                                     |
| position model | the pair contract **is** the LP token — mint/transfer/burn on the pair itself                                                              |

The position model is the cheap one here: LP tokens are already indexed as
assets, so holders come from `balances` with the usual dedup.

## Four-oracle table — fill before implementing

| #   | Oracle                                                       | Status                                                                                                                                                                                                                               |
| --- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1   | Soroswap's own API                                           | **EXISTS, behind an API key** (probed 2026-09-02: `api.soroswap.finance` live, Swagger at `/docs`, `/pools` answers 403). Usable once a key is obtained; until then the independent volume check falls to #2/#3 — stated scope risk. |
| 2   | pair contract via RPC (`get_reserves`, `token_0`, `token_1`) | available                                                                                                                                                                                                                            |
| 3   | checkpoint snapshots                                         | available; the only historical oracle                                                                                                                                                                                                |
| 4   | independent aggregator                                       | **unreliable for this protocol** — one major aggregator reports it at protocol level with a per-pool feed that omits it entirely. Do not use as a coverage check.                                                                    |

## Pre-adapter probes (2026-09-02) — the decisive seams are already settled

Probed live before any code, after the 0517 rule made the events readable:

1. **Reserves live in the pair's OWN instance storage** — read verbatim from
   mainnet (`stellar contract read` on pair `CAM7DY53…`, the native-USDC
   pair): Soroswap's `DataKey` is a plain enum, so instance keys are u32
   DISCRIMINANTS, not symbols: `0` = token0 address, `1` = token1 address,
   `2`/`3` = Reserve0/Reserve1 as `i128`, `4` = the FACTORY address
   (`CA4HEQTL…7AW2` — exactly the archived factory), plus SEP-41
   `METADATA`/`TotalSupply`/`name`/`symbol` — the pair IS the LP token, with
   on-chain metadata ("native-USDC Soroswap LP Token").
   **Consequences:** (a) the reserve source is self-authenticated ledger
   STATE (owner == pool) — the ADR 0058 doctrine fits with no new tables;
   `sync` demotes to the monitored cross-check, exactly `update_reserves`'
   fate; (b) the pair's own factory pointer gives the registration
   corroboration the same class of authenticity as Aquarius's `Router` key;
   (c) the parser needs a u32-discriminant reader (not symbol-keyed) — the
   discriminants must be anchored in vendor source per deployment WASM, per
   the umbrella's shape-not-name rule.
2. **True-swap flow re-measured — Soroswap first** (recorded in 0516):
   48,334 vs Phoenix 5,526 per 1M fresh ledgers (8.7x), both eras ahead.
3. **`new_pair` shape fetched from current vendor source** (factory
   `event.rs`): topics `[String("SoroswapFactory"), Symbol("new_pair")]`
   (0517 arm 2 → signature `new_pair`), data
   `{token_0, token_1, pair, new_pairs_length}` — the registration carries
   the pair address in DATA from an authenticated emitter, and
   `new_pairs_length` is a free monotone counter for the discovery
   closure check. Discovery counts to reconcile: 191 label-emitting pairs
   (measured, = the 0516 "trading" count) vs 253 `…Soroswap…` metadata
   names (drift from 248) vs factory `all_pairs_length` (RPC).
4. **Vendor API (oracle #1) EXISTS but is keyed**: `api.soroswap.finance`
   is live (NestJS), `/docs` serves a Swagger UI, and `/pools` answers
   **403 Forbidden** — the endpoint exists behind an API key. Scope line
   for the four-oracle table: oracle #1 available IF a key is obtained;
   otherwise the independent volume check falls to the #2/#3 oracles and
   must be stated as the 0516 scope risk.

## In-DB completeness PROVEN + decisions (karolkow, 2026-09-02)

Measured before implementation, settling the backfill question with evidence:

- **Four factory deployments** share the `SoroswapFactory` label: the
  documented `CA4HEQTL…7AW2` (214 pairs, counter 1→214 GAPLESS,
  50.75M→63.8M) plus three dead early deployments (11/6/4 pairs, eras
  50.69-50.75M, each counter complete 1→N) — the Aquarius ten-routers story
  again. 235/235 registrations, 235 distinct pairs, no pair registered
  twice.
- **`min(new_pairs_length) = 1` per factory** — every factory's FIRST pair
  postdates the ingest floor; no pre-floor gap exists for this protocol.
- **Orphan emitters: 0** — every `SoroswapPair`-labelled emitter in history
  is in the registered set.
- Therefore: registry (identity + legs from `new_pair` data), reserve
  history (`sync`, absolute values) and volume (`swap`) are ALL fully
  derivable in-DB. **Decision 63: backfill is in-DB (option A)**, and the
  closure check comes free from the vendor's own monotone counter:
  `max(new_pairs_length) == count()` per factory — stronger than set
  reconciliation.
- **Decision 64: `pool_type_raw` stays `''`** — the vendor emits no type
  (one fixed constant-product mode); an invented label would be our
  interpretation, not a verbatim value.
- Reserve SOURCE per the probes above: live = the pair's own instance
  storage (self-authenticated; `plane_id` = the pair's own id — stamp,
  declaration and owner coincide); history = `sync`-derived rows; the
  live/history seam is verified by the bidirectional anti-test on the
  overlap. `sync` thereafter stays the monitored cross-check.
- Minor, recorded: current `total_shares` for DORMANT pairs is the one
  value not in the DB — the live writer fills it on a pair's next
  activity; an optional one-shot RPC pass covers the rest.

## Write path implemented + local e2e record (2026-09-02)

Branch `feat/0518_soroswap-pool-adapter`, five commits. Shape: new parser
module `pool_soroswap.rs` (the u32-discriminant instance reader is
deliberately separate from the symbol-keyed Aquarius one) + stage arms into
the SAME three tables and folds — adding the protocol touched no shared
table shape, the 0516 umbrella promise holding in practice.

- **Registration**: `new_pair` corroborated by the pair's own factory
  pointer (DataKey 4) AND instance CREATION in the registering ledger; no
  UNVERIFIED arm (the pointer is part of the recognition shape). Raw-ledger
  e2e over three eras / both factory generations validated the same-tx
  creation assumption on real history.
- **State**: owner, stamp and declaration coincide — `plane_id` and
  `share_token_id` are the pair's own id; `total_shares` from the SEP-41
  `TotalSupply`.
- **Verified on a full 64k-ledger real partition (55.36-55.42M) through the
  real runner into a fresh DB**: structural 1,563/1,563 one-row-per-key,
  self-property 11/11, multi-plane monitor 0, bidirectional sync↔state
  anti-test **1,563/1,563 values equal with zero remainders both ways**
  (this is what makes decision 63's history-from-sync seam sound), and a
  production cross against the independent old-code ingestion **1,563/1,563
  exact**.
- **The e2e caught a real dialect bug no unit test could**: `TotalSupply`
  rides a VEC-WRAPPED sym key (token-SDK enum encoding) while `METADATA` is
  bare — the fixture came from a stellar-CLI dump whose dialect flattens
  the wrap, so supply read 0 on every active pair. Fixed, re-proven on a
  re-indexed slice: the recovered value equals the raw ledger to the unit
  (2,070,830,028,682 on the native-USDC pair).
- Backfill runbook: `docs/backfills.md` § "Soroswap pool passes" — both
  passes are in-DB (no S3) via small Rust one-offs (surrogates are not SQL-
  computable), closure from the vendor's own counter.

## Deferred by decision (karolkow, 2026-09-02): one write-seam at the THIRD protocol

Today each family rides its own `ParseOutput`/`StageInputs` field
(`pool_instances`, `factory_pairs`) — N families, N fields, every seam
compiler-forced by the exhaustive destructures. Decision 4a: DO NOT unify
now; when the Phoenix adapter lands (third family), collapse the per-family
vectors into one `Vec<PoolFamilyWrite>` enum seam and let stage match on
the variant — the rule of three, and the depth-first principle's own
mechanism ("the next protocol updates the model when its turn comes, and
the diff then shows exactly what differed"). Recorded here AND in the
wayfinder map so the Phoenix task inherits it as a step, not a rediscovery.

## Acceptance Criteria

- [x] four-oracle table completed with evidence, absences stated (#1 keyed, risk stated)
- [x] deployments enumerated by shape and classified (4 factories, 3 dead early)
- [x] pools discovered, reconciled against an independent count (vendor counters gapless 1..N per factory; 0 orphan emitters)
- [x] reserves — SUPERSEDED source per the probes: the pair's own instance STATE (self-authenticated); `sync` demoted to the monitored cross-check and proven value-identical 1,563/1,563 both directions in the local e2e
- [x] volume from `swap` — already in `soroban_events` with signatures post-0517; read-half consumes it (no write-side work, depth-first)
- [ ] holders from `balances` on the pair asset, deduped — READ-HALF (mechanism settled: the pair IS the LP token)
- [ ] unioned into the pool list under its own deployment label — READ-HALF (deferred to the end of the roadmap by owner decision)
- [x] backfill in-DB, no S3 re-parse — runbook written; execution is the deploy window's

## Third adapter in the SAME PR: the config-factory family (2026-09-03)

Owner decision: PR #447 does not merge alone — the Phoenix-family adapter
rides the same PR. Naming decision 70:A — mechanism name
`pool_config_factory` (pools whose state lives under a `CONFIG` key);
order decision 71:A — the 4a seam refactor lands FIRST, the adapter on the
clean seam.

### Decision 4a EXECUTED (commit `9ae66b27`)

The three parallel family slices (`plane_pool_data`, `pool_instances`,
`factory_pairs`) collapsed into one `Vec<PoolFamilyWrite>` enum threaded
extraction→staging; stage partitions by variant. No behavior change — 646
tests + both raw-ledger staging e2es unchanged. Adding this third family
was then a variant + an arm, proving the seam's point on day one.

### Research settled on real data before code (all measured)

- **Discovery**: ONE factory in all of history (`CB4SVAWJ…`, its own
  `initialize` event at 51,572,024 is IN our events — coverage from
  birth). Registration event `[String("create"), String("liquidity_pool")]`
  with a bare pool address as data — no tokens, NO counter. 14 events,
  14 distinct pools.
- **State**: decoded the raw creation ledger (64,030,567). Per-key
  PERSISTENT entries on the pool: `CONFIG` sym → map (token_a, token_b,
  share_token, stake_contract, pool_type u32, total_fee_bps i64 —
  per-pool fee, 50 on the newest), u32 discriminants `0`=TotalShares,
  `1`/`2`=ReserveA/B (i128), `3`=Admin, plus an `XYK_POOL` marker; the
  contract INSTANCE itself carries no storage. Everything written in the
  registering transaction (created gate holds; reserves/shares are TRUE
  zeros).
- **No factory back-pointer exists** (unlike the pair family's DataKey 4),
  so corroboration = created gate + the pool's own full CONFIG in the
  registering ledger. Shape-not-brand: no hardcoded factory address.
- **Reserve co-occurrence (the one open risk) closed decoder-independently**:
  raw-JSON walk over 10 pool-transactions across three eras (52.0M, 58.0M,
  64.25M) — ReserveA and ReserveB always written together (TotalShares
  joins on provide/withdraw). Both-or-neither is a measured rule, not an
  assumption.
- **Stable pools**: none on mainnet (0/14). The stable contract source
  shares the key names; the reader takes `pool_type` from CONFIG, so the
  first stable pool flows through with its own discriminant.

### Design decisions

- `pool_state_changes` rows are self-stamped (`plane_id` = pool's own id) —
  owner, stamp and declaration coincide, same construction as the pair arm.
- **`pool_instance_state` row ONLY when the tx wrote CONFIG** (creation +
  admin changes): the table is RMT whole-row keyed on `pool_id`, and a
  per-op TotalShares write arrives config-less — staging it would clobber
  `share_token_id` to 0 (the misleading-fallback class). Consequence,
  recorded for the read half: for THIS family `total_shares` in the table
  is a config-write-time snapshot; the live supply is the share token's
  own tracked supply (a separate SEP-41 contract on the generic pipeline).
- `pool_type_raw` stores the vendor `PairType` discriminant verbatim ("0");
  `fee_bps` from CONFIG's per-pool `total_fee_bps` (creation-time snapshot,
  mutable via `update_config` — same caveat as both siblings).

### Evidence so far

- Parser module `pool_config_factory.rs`: 9 unit tests on verbatim mainnet
  payloads (CONFIG map, registration event, grouping, half-pair refusal,
  foreign-contract sieve).
- Stage cross tests: corroborated registration stages the pool's own facts;
  unconfigured/touched refused; config-less op write stages reserves and
  does NOT clobber the declaration. 655 tests green, clippy clean.
- `config_pool_real_corpus`: 14/14 registrations decode, 0 duplicates;
  keyed-entry sieve over a rebuilt 32-raw-ledger corpus (14 registration
  ledgers + 3 eras of swap ledgers + a 10-ledger hot window): 27
  extractions, **0 foreign** — FP=0. (The old shared 79-ledger corpus dir
  was session-scratch and is gone; the new dir is
  `config_pool_corpus_dir`, recipe in the test docs.)
- `config_pool_stage_real_e2e`: **the entire registration population**
  (14/14 ledgers, 4 pool-wasm generations) through full staging — every
  registration corroborates (created + CONFIG) and stages registry +
  reserve + declaration rows with the pool's own facts.

### Discovery: the factory's live list is MUTABLE — closure is ⊆, not =

RPC `query_pools()` returns 13 pools; history has 14. The missing one
(`CAZ6W4WH…`, third-ever registration, 25,873 events, traded until 63.77M)
was DELISTED from the factory's vector but is a real pool with real
history. Event-append-only discovery is therefore the correct source, and
the backfill closure check must be **RPC ⊆ ours**, never set-equality. (A
query_pools-seeded registry silently loses this pool's entire history.)

### Full-runner e2e record (2026-09-03) — third adapter proven end-to-end

Fresh DB from the branch `init.sql` (35 tables) in the local docker CH; the
RELEASE runner (production write path, all families in one pass) over two
real slices:

- **Registration slice** 64,030,400–64,030,700 (301 ledgers): the
  `CCPPPTDW…` registration staged exactly — registry row `pool_kind=1,
fee_bps=50, pool_type_raw="0"`, `pool_id` equal to the contract payload
  byte-for-byte, declaration row with the SEPARATE share token and
  `total_shares=0`, creation reserve row `[0,0]` self-stamped.
- **Swap slice** 64,164,300–64,167,400 (3,101 ledgers): **36/36
  (pool, ledger) keys — exact set equality with production events**, per
  pool 22+12+1+1 with matching first/last ledgers; ONE `plane_id` across
  all rows (the pool's own surrogate); spot value check against the raw
  ledger 64,167,385: staged `[145735138754, 25897770547]` equals the
  entry's `updated` values exactly.
- Operational note for the deploy-window backfill: the NEWEST 64k S3
  partition can lag (observed 63,505/64,000 files — the runner skips it
  with a warn); slices must stay behind the last complete partition.

### RPC leg for the pair family too (2026-09-03)

Owner asked every family to carry a live-RPC check. Two most-active pairs:
`get_reserves` + `total_supply` invoked on the contract vs the pair's raw
instance entry fetched via `getLedgerEntries` and read by our key scheme
(u32 2/3, VEC-wrapped `TotalSupply`) — **6/6 values equal to the unit**,
and the vec-wrap key shape confirmed live. All three families now have a
chain-RPC verification leg (router: 26/26 reserves in 0374; config-factory:
`query_pools` + raw creation ledger).

## Deep multi-agent review of PR #447 + all findings executed (2026-09-05)

Six sequential agents (correctness, simplify, devil, prod-readiness,
security, architect) over the full branch diff + 1-2-hop dependents, with a
judge pass (dedup, adversarial re-verification of every P2 in code,
ADR-quote test on downgrades, pattern-generalization grep). Verdict:
**APPROVE WITH CHANGES**; architecture **BETTER** (the 4a seam passed the
deletion test — the third family cost 2 production files instead of ~15).
All findings were then executed:

- **M1 (P2, three agents converged independently):** the config-factory
  arm bound nothing to the event's EMITTER (the family has no back-pointer
  to check), so a second emitter co-claiming a genuine pool inside its
  creation ledger would stage a duplicate registry row at the same RMT
  version — nondeterministic `deployment_id`. An agent's attempted
  downgrade via the shape-not-brand rule FAILED the quote test (that rule
  sanctions self-description; this corrupts a genuine pool's attribution).
  Fixed: conflicting emitters for one pool now refuse BOTH loudly;
  identical duplicates collapse to one row; cross test added; the module's
  forgery analysis now names all three shapes.
- **Pair-gate hardening:** the registration gate now also compares the
  event's legs against the pair instance's OWN token_0/token_1 (claim vs
  ledger-authenticated authority) — mismatch refuses.
- **Runbook (executed before the deploy window can run the passes):**
  Soroswap pass 1 gained a MANDATORY RPC corroboration leg (events-only
  backfill accepted what the live gate refuses) and pass 1 now emits the
  `pool_instance_state` declarations too (the old "no history pass" text
  contradicted the ADR 0058 read rule — dormant pairs' whole reserve
  history would be invisible); closure gained an EXTERNAL anchor per
  family (live `all_pairs_length` per factory; pinned count 14 for the
  config family) + a `DB::Exception` grep on every chq harvest (chq exits
  0 on server errors — a truncated corpus self-certifies); the config
  family gained a standing live cross-check (per-release RPC reserve
  comparison + executable_update watch) as the one family with no event
  oracle.
- **Wording/comments:** "config-write-time snapshot" → "structurally 0
  forever" (code, schema overview, runbook, init.sql column comment);
  init.sql's fee_bps/pool_type_raw comments now describe all three
  families; instance-refusal logs no longer overclaim ("pair write
  refused" → "pair instance row refused (reserve row already staged)").
- **Dedup:** shared `parse_reserve_pair`/`parse_supply` beside
  `parse_reserves`; new `xdr_parser::meta::for_each_tx_meta` (exhaustive
  V0/V1/V2, same philosophy as `ledger_changes`) adopted by the five
  raw-ledger tests in this PR — the two PRE-EXISTING production unrolls
  (`envelope.rs`, `ledger.rs`) and older test files are left for the 0525
  convention pass (minimal-diff rule).
- **Deferred to the next stage.rs touch (architect, sanctioned by the
  0485/0525 precedent):** extract the ~620-line pool staging block into
  `persist/stage/pools.rs` as a pure move — recorded here so 0525 inherits
  it as a step.

Review checklist per the task template:

- [x] **Docs updated** — `docs/architecture/{database-schema,indexing-pipeline,xdr-parsing}/*-overview.md` + `docs/backfills.md` updated in this PR (ADR 0032)
- [x] **API types regenerated** — N/A: no `crates/api/**`, `Cargo.{toml,lock}` or `libs/api-types/**` paths in the diff (verified by --name-only)

### Gate upgrade to TWO-STAGE (owner request, 2026-09-05): pointwise first

Owner challenged refuse-both: "why refuse both instead of only the invalid
one?" Probes settled the design space first: the pool address's deployer
preimage (the cryptographic anchor tying pool → factory) is NOT present in
the ledger meta (checked on the raw creation ledger — no
`contract_id_preimage`/salt anywhere, the factory-internal deploy leaves
no envelope/auth trace); and the factory-list idea alone does NOT
discriminate (an attacker can copy the pool's address into HIS own list —
addresses are just data). Hence the two-stage gate:

1. **Membership list, POINTWISE** — the emitter must have written the pool
   into an address list in storage the EMITTER owns, this ledger (the
   genuine factory appends each pool to its own pools vector — probed on
   raw meta, key u32(2), and the sieve is deliberately key-agnostic:
   `extract_address_list_writes`, new `PoolFamilyWrite::AddressList`
   variant). An event-only forgery now dies alone and the GENUINE
   registration survives with the genuine attribution.
2. **Conflict backstop** — two corroborated claimants (the determined
   attacker) still refuse BOTH; no ledger fact arbitrates them.

Proven: the membership-list anchor holds on the ENTIRE registration
population (14/14 ledgers, e2e asserts owner==emitter ∧ list∋pool across
every factory-wasm generation); 664 tests + all real-ledger suites green.
Runbook updated: the registration backfill drives the same staging gate,
never the row builder directly.

## External cross-verification against the sibling registry — and a completeness catch (2026-09-05)

Owner asked to verify our work against the sibling project's tables (same
ClickHouse, `prices.*`, their registry seeded independently from the
vendor's API — a genuinely external source). Results, per venue
(set-compared by address):

- **Aquarius**: theirs 488 ⊂ ours 497 exactly (0 only-theirs; our 9 extra
  = the UNVERIFIED router-less legacy + newer-than-their-seed).
- **Soroswap**: theirs 221 ⊂ ours 235 exactly (0 only-theirs; our 14
  extra = dead-factory pairs + newer-than-their-seed).
- **Config-factory family: the cross-check CAUGHT A GAP.** They carried 6
  pools our harvest lacked — five DEAD EARLY factory deployments
  (50.87M–51.57M, before the documented factory's own initialize) emitted
  the same registration shape; the first corpus harvest was scoped to the
  documented factory because the shape-wide query had timed out. Exact
  materialization of the review's "self-referential closure" finding.
  Closed the same day: full-history shape-wide sweep in slices (which
  itself hit the HOURLY READ QUOTA and truncated silently once — the
  `DB::Exception`-grep rule caught it, second review finding proven live)
  → definitive population **20 registrations / 6 factory deployments /
  nothing newer to 65.4M**; the 6 ledgers joined the reg-ledger set and
  corpus; **20/20 through full staging incl. the two-stage gate — the
  membership-list anchor holds on the earliest era (50.87M)**; corpus
  20/20 across 6 factories, sieve FP=0 over 38 raw ledgers; docs/comments
  corrected 14→20 with the lesson recorded in the runbook.
- Their seed is STALE in the other direction: they lack `CCPPPTDW…`
  (newest pool, created 64,030,567) — worth a line in the handoff so they
  reseed.

Net: on every venue their independently-sourced registry is a strict
subset of ours, and the one asymmetry it exposed was OUR harvest scoping,
not the detector (the live shape-driven detector would have caught the
dead factories all along — the corpus, docs and pinned closure count were
what lagged).

### Deficit attribution, exact (2026-09-05 follow-up)

- **Soroswap, their 14 missing = vendor counters 201–214** — the 14 newest
  pairs of the documented factory (ledgers 63.40M–63.80M), nothing else:
  all 21 dead-factory pairs ARE in their registry (the vendor API lists
  them). Pure stale seed.
- **Aquarius, their 9 missing = 6 `concentrated` + 2 `stable` + 1
  `constant`.** The 6 concentrated are their seeder's DELIBERATE hold-back
  (their own seed code maps `concentrated` to None pending event-shape
  verification); the other 3 registered 63.40M–64.00M = newer than their
  seed (estimate on the boundary).
- Coherent seed-date estimate across all three venues: their last seed ran
  around ledger ~63.3–63.4M (Phoenix has 63.29M's pool but not 64.03M's;
  Soroswap missing starts at 63.40M).
