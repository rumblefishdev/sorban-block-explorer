---
id: '0516'
title: 'Soroban AMM coverage: shared pool model + four-oracle validation method'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0374', '0517', '0518', '0008']
tags:
  [
    backend,
    clickhouse,
    api,
    liquidity-pools,
    umbrella,
    priority-medium,
    effort-large,
  ]
links:
  - 'https://github.com/rumblefishdev/soroban-block-explorer/issues/405'
history:
  - date: '2026-08-26'
    status: backlog
    who: karolkow
    note: >
      Umbrella for multi-protocol AMM coverage. Owns the shared model and the
      validation method so each adapter is small; 0374 (Aquarius) is the first
      adapter and predates it. Prior art in archived 0008.
---

# Soroban AMM coverage — umbrella

## Summary

One pool model and one validation method for every Soroban AMM we index, so
that adding protocol N+1 is an adapter and not another redesign. This task owns
what is shared; each protocol gets its own task for what is not.

## Context

`/liquidity-pools` is fed by the classic protocol only. Soroban AMMs are a
separate world with their own pool contracts, and there are three that matter.
That list is not our guess — the Soroswap aggregator, whose business is to know
every Stellar AMM, integrates exactly those three. Measured 2026-08-27:

| Family                                   |                     Contracts |   Swap events |    Share |
| ---------------------------------------- | ----------------------------: | ------------: | -------: |
| Router-registry family (Aquarius-shaped) | 371 trading of 497 registered |     4 363 284 |     63 % |
| **Phoenix**                              |                        **14** | **1 969 860** | **29 %** |
| Soroswap                                 |      191 trading of 232 pairs |       578 921 |      8 % |

**Correction 2026-09-02: the event counts above are RAW EVENTS and mislead on
flow.** Phoenix publishes 6-8 events per swap (one per field), Soroswap one —
so per TRUE swap Soroswap leads in both eras (48,334 vs 5,526 in a fresh
1M-ledger window; details in the adapters section below). The original note
("Phoenix is second") stood on this artefact; the Soroswap-was-invisible
observation stays true (its events decoded to nothing before 0517), but the
ranking it produced is reversed.

**Read [0008](../archive/0008_RESEARCH_event-interpreter-patterns/README.md)
before starting.** It already documents Soroswap's and Phoenix's event shapes
from their own source, and a pattern-registry design. That research was
archived and never reached the parser — the same event shape was rediscovered
from raw data on 2026-08-26. Preventing that second discovery is half the point
of this task.

## The four-oracle rule

Run this per protocol. Record every line as present or absent — never blank.

| #   | Oracle                          | Validates                                | Covers history? |
| --- | ------------------------------- | ---------------------------------------- | --------------- |
| 1   | the protocol's own API          | discovery, pool type, fee, volume        | no              |
| 2   | the contract via RPC simulation | current state; never our decode path     | no              |
| 3   | checkpoint snapshots            | **the only oracle for historical state** | yes             |
| 4   | an independent aggregator       | coarse protocol-level sanity             | no              |

Two rules learned from the Aquarius pass:

- **A protocol with no #1 leaves only #2 and #3** — current state and history,
  no independent volume check. State that in the adapter task as a scope risk.
- **The vendor's API defines the vendor's scope.** Measured: the Aquarius API
  lists one router's pools and has zero overlap with the nine other routers
  emitting the same events. Do not attach a brand to pools the vendor's own
  catalogue excludes.

`stellar.expert` is not an oracle here — its pool API returns classic pools
only. An aggregator's per-pool feed may omit a protocol entirely even while
reporting it at protocol level; check before relying on it.

## Anchor every decoder in the vendor's own source — and record what you could not reach

Soroban defines no AMM standard, so there is no specification to decode
against. That makes the vendor's own source the highest authority available,
and observation the arbiter of last resort. Before writing any decoder:

1. **The vendor's contract source.** Their event-emitting code states the
   payload exactly. Archive the snippet into the task — repositories go
   private (Aquarius's did, between one research pass and the next).
2. **The deployed contract's own spec, pulled from chain.** Live, signed by
   deployment, and independent of any repository:
   `stellar contract info interface --id <contract> --rpc-url …`.
   **It carries function and type definitions but no event definitions**, so
   it can corroborate types and lookup keys and can never confirm an event's
   shape. Use it for what it covers, and say so.
3. **Structural conformance across all history.** Count how many real events
   match the decoder's checks. Anything short of 100 % is a finding.

**Record which of the three you actually reached.** "Verified against the
vendor's source" and "verified against a five-month-old capture that the
vendor has since taken private" are different claims, and the second one is
what we usually have.

Expect vocabularies to disagree between sources rather than to align. One pool
type carried three spellings at once: `constant` in the router's event,
`standard` in pool state, `ConstantProduct` in the contract's own enum — and a
fourth shape (`elastic`) exists on chain that appears in none of them, because
it belongs to a different deployment's code.

## Event names collide across protocols — the name is never the identifier

`withdraw_liquidity`, `provide_liquidity` and `swap` are each emitted by more
than one unrelated protocol. A decoder keyed on an event name will claim
another protocol's events. Key on the **full shape** — topic types, data types,
arity — and measure the false-positive rate against all history before
trusting it.

Three layers keep a decoder honest, and all three are cheap:

1. **Shape, not name.** A specific topic/data layout is a far narrower sieve.
2. **Behaviour confirms a claim.** A registry entry is a _candidate_; the pool
   is real once it emits pool activity. Measured on the router family: 23
   registrations from five dead deployments never emitted anything at all.
3. **Unknowns are counted, never guessed.** An unrecognised value yields
   `None` plus the raw string plus a monitored counter.

## Deployment is the unit, not the brand

Follow the convention DEX front-ends already use, where a protocol appears as
its distinct deployments rather than one merged name. Each router/factory
deployment is its own entry in the protocol dimension and its own filter value.

This is not cosmetic. It bounds implementation scope to the deployment the
vendor documents — which is exactly the scope their API can validate — and it
keeps unidentified deployments from being labelled with someone else's brand.
Deployments with no live pools are indexed for historical completeness and get
no listing entry.

## Shared model — built once, here

1. **Pool identity** widens to a contract address; the classic `L…` strkey
   stays valid. One id column, two shapes.
2. **Legs are an ordered list**, not `asset_a` / `asset_b`. Three-leg pools
   already exist.
3. **Amounts are raw `Int128` plus a decimals column.** An 18-decimal leg is
   already on chain and the classic scale would corrupt it. A leg whose
   decimals cannot be resolved renders with an explicit marker, never a
   plausible wrong number.
4. **Protocol + deployment** as a first-class dimension on every pool row.
5. **Position model as an enum** — fungible share token, ranged position, or
   trustline. Not every protocol has fungible shares; a pool whose positions we
   cannot enumerate must say "not indexed", never show an empty holder list.

## The four seams — all that an adapter may vary

Everything else is shared. If a protocol needs a fifth seam, that is a signal
to change the model here rather than to special-case it in the adapter.

| Seam                       | Known variants                                                 |
| -------------------------- | -------------------------------------------------------------- |
| discovery                  | registry event on a router · self-announcing pair contract     |
| where the event name lives | `topics[0]` · `topics[1]` behind a protocol label              |
| reserve source             | dedicated event · contract state entry                         |
| position model             | separate share token · ranged position · pool is its own token |

## Per-protocol adapter checklist

Copy into each adapter task:

- [ ] four-oracle table filled in, each line present or absent
- [ ] deployments enumerated by shape, each classified with on-chain evidence
- [ ] scope fixed to one deployment, named after it
- [ ] the four seams answered
- [ ] discovery reconciled against an independent count, not against the
      registry that produced it
- [ ] reserves compared to the contract's own answer, across every pool type
- [ ] positions: coverage measured, unresolvable cases render "not indexed"
- [ ] Aquarius concentrated positions (0374's deferred step 23): index from
      `position_update` per the decision recorded in 0374's notes
      (2026-08-27), then lift the participants endpoint's explicit 400 for
      concentrated pools
- [ ] backfill is in-DB where the raw columns already hold the data

## Adapters

| Protocol               | Task                                                                         | State                                     |
| ---------------------- | ---------------------------------------------------------------------------- | ----------------------------------------- |
| Router-registry family | [0374](../active/0374_FEATURE_lp-native-leg-and-soroban-amm-completeness.md) | active, first adapter                     |
| **Soroswap**           | [0518](./0518_FEATURE_soroswap-pool-adapter.md)                              | **next after 0374** (0517 fix in PR #443) |
| Phoenix                | —                                                                            | after Soroswap; spawn then                |

**Order reversed 2026-08-27 on measurement — and REVERSED BACK 2026-09-02 on a
better one (karolkow).** The 3.4x figure counted raw EVENTS, but Phoenix
publishes 6-8 events per swap (one per field) while Soroswap publishes one —
the ratio was an artefact of the publishing convention. Counted per TRUE swap
(the one `sender` event per Phoenix swap vs Soroswap pair `swap` rows):
Soroswap leads **48,334 vs 5,526 (8.7x)** in a fresh 1M-ledger window and
45,135 vs 23,499 (1.9x) in a historical one — Soroswap ahead in both eras and
the gap GROWING. It also wins on decode difficulty (one struct vs cross-event
correlation), on discovery evidence in hand (factory + pair storage probed,
see 0518), and it is what issue #405 asks for by name.

## Sibling recon — stellar-prices-api (develop, read 2026-09-02)

The prices project indexes the SAME three venues for OHLCV, events-only
(no ledger state, `sync` skipped, no reserves). Their archive is a paid-for
trap list for exactly our next steps; recorded here so no trap is walked
twice. Their design also VALIDATES ours by contrast: multiple silent-zero
incidents trace to events-as-the-only-source (their 0096: 536k Soroswap
swaps → 0 candles, 0 alerts, because decoder AND guard keyed the same
wrong topic) — our state-first-with-event-cross-checks stands.

**Traps recorded there that hit OUR roadmap:**

- **Phoenix grouping (their 0097/0099)**: group per-field events by
  contiguity per (transaction, contract) and validate by PRESENCE of the
  four required fields (sell_token, offer_amount, buy_token,
  return_amount), capped at one swap's worth — NEVER by event count: a
  `len >= 8` gate silently discarded 5,175 real 7-event swaps (~2.1%; the
  `actual received amount` field is optional). Liquidity groups reject on
  absent required fields.
- **Phoenix has TWO XYK WASM hashes with identical interfaces** (their
  0032/0034) — keying dispatch or discovery on wasm_hash silently drops a
  pool family member. Phoenix stable pools: zero on mainnet, they keep a
  periodic re-survey instead of dead code.
- **Aquarius router `swap` SUMMARY events** (their 0087): recognizable by
  an address-Vec at topic[1]; counting them alongside pool `trade` events
  double-counts volume. Any event-side cross-check of ours must
  discriminate the same way.
- **Soroswap `/pools` API**: bearer-key auth, in ACTIVE use by that team —
  the oracle-#1 key for 0518 exists in-house; ask them before asking the
  vendor. Their registry-seed from it stayed a stopgap (current-set-only,
  misses dead pools) — event discovery remained necessary.

**Adopted into our method (0516-level):**

1. An `unresolved_pools`-style GUARD TABLE: any pool-shaped activity from
   a contract absent from the registry gets a row (contract, venue guess,
   count, ledger range), with the invariant "empty after a clean forward
   run". Generalises our per-vendor closure checks to every venue and
   future factory. CRITICAL amendment from their 0096 post-mortem: the
   guard's shape predicate must be derived INDEPENDENTLY of the decoder's
   (theirs shared it, so one bug blinded both).
2. Their live/backfill single-seam rule confirms ours (backfill-runner and
   the indexer already share `parse_ledger` by construction) — keep it a
   stated invariant.
3. Real-sample fixtures for every extractor (their `dump-swap-events`
   tool) — already our house practice; keep matching it per venue.

## Acceptance Criteria

- [ ] shared model items 1–5 implemented and used by at least two adapters
- [ ] four-oracle table filled in for every adapter task
- [ ] protocol + deployment filter on the pool list
- [ ] adding an adapter touches no shared table shape
- [ ] account-page display decided (LAST, once positions are indexed): how a
      holder's LP participation shows on the account page — concentrated
      positions AND classic `lp_positions` (neither is visible there today;
      fungible soroban share tokens already appear in the Assets card as
      plain tokens). Direction recorded 2026-09-02: a separate "Liquidity
      positions" section, never rows squeezed into the Assets card — a
      position is not a token balance; no valuation until a portfolio-value
      feature exists and the amount conversion passes an on-chain check.
