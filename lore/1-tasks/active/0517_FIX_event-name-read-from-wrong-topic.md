---
id: '0517'
title: 'Event name is read from topics[0]; protocols that label there lose it'
type: FIX
status: active
related_adr: []
related_tasks: ['0516', '0518', '0008']
tags: [backend, xdr-parser, clickhouse, priority-high, effort-medium]
links: []
history:
  - date: '2026-08-26'
    status: backlog
    who: karolkow
    note: >
      Measured on production while scoping 0516. Correctness bug for all
      Soroban events, not only AMMs — filed separately so it is not gated on
      AMM work. Shape was documented in 0008 before the parser was written.
  - date: '2026-09-02'
    status: active
    who: karolkow
    note: >
      Activated after 0374's write half shipped. Pre-implementation research
      re-measured the NULL population on two 1M-ledger windows: the planned
      sym-fallback rule alone would miss the whole Phoenix family (both
      topics are String there), and a NEW label-convention protocol
      (BlendStrategy) appeared. Rule extended to four arms; the protocol
      label stays in topics_xdr (extract on demand, never copy).
---

# Event name read from the wrong topic

## Summary

We take an event's name from `topics[0]`. Some protocols put a protocol label
there and the name in `topics[1]`. For those, `signature` lands `NULL` and the
event is unusable — silently, with no error anywhere.

## Measured on production, 2026-08-26

Chain-wide, 3 816 728 events carry a `NULL` signature — 0.04 % of 10.3 G. That
looks negligible until it is grouped by emitter:

| Emitter label   | Event                                                     | Rows    |
| --------------- | --------------------------------------------------------- | ------- |
| `SoroswapPair`  | `sync`                                                    | 572 576 |
| `SoroswapPair`  | `swap`                                                    | 570 858 |
| `SoroswapPair`  | `deposit`                                                 | 1 369   |
| `SoroswapPair`  | `withdraw`                                                | 348     |
| `SoroswapPair`  | `skim`                                                    | 23      |
| `DeFindexVault` | `deposit` / `withdraw` / `rebalance` / `rescue` / `dfees` | 674     |

**1 145 174 rows — 30 % of every undecoded event on the chain — are one
protocol.** The decoded topic vector shows why:

```
topics = [ String("SoroswapPair"), Symbol("sync") ]
              ^ we read the name here    ^ the name is here
```

`sync` carries the pool reserves after every trade. So an entire protocol's
reserve history is already on disk and unreadable.

## This was known

[0008](../archive/0008_RESEARCH_event-interpreter-patterns/notes/R-dex-swap-event-signatures.md)
records the shape from Soroswap's own source:

```rust
e.events().publish(("SoroswapPair", symbol_short!("swap")), event);
```

It also covers Phoenix. The research was archived and the parser was written
against the other convention anyway.

## Implementation

- Name resolution takes `topics[0]` when it is a `Symbol`; when it is not,
  fall through to `topics[1]` and keep `topics[0]` as the emitting protocol
  label. Both are worth storing — the label is a free protocol discriminator.
- Nothing may be dropped silently. An event whose name resolves nowhere keeps
  `NULL` **and** increments a monitored counter, so the next convention shows
  up as a number rather than as absence.
- Backfill is in-DB: `topics_xdr` already holds the decoded vector, so this is
  `INSERT … SELECT`, not an S3 re-parse.

## Pre-implementation research (2026-09-02) — the plan's rule was insufficient

Re-measured the NULL population on two 1M-ledger windows (fresh 63.3M+,
historical 55-56M; shapes identical in both). Three findings that changed
the implementation:

1. **The planned sym-fallback alone would miss the whole Phoenix family.**
   Phoenix publishes `("swap", "sender")` as plain `&str`s — BOTH topics are
   String, so "take the Symbol from topics[1]" resolves nothing there. The
   family (`swap` 188k in the historical window, plus `bond`/`unbond`/
   `provide_liquidity`/`withdraw_rewards` from their stake contracts) needs a
   third arm: a String first topic with no Symbol second IS the name.
2. **A new label-convention protocol appeared: BlendStrategy** — 67k rows in
   the fresh window, more than DeFindex. The convention is spreading, which
   is what the monitored-counter AC is for.
3. **100% of the measured NULL population has a String first topic and zero
   have empty topics** — so the unresolved-warn arm is quiet today and any
   noise from it is a genuinely new convention.

**Decision (karolkow): the protocol label is NOT lifted into a column** —
it sits verbatim in `topics_xdr` forever; extract on demand, never copy
(the subpool_salt rule). Revisit only if 0518's discovery design measures a
need for the filter.

## Implementation notes

- `extract_event_signature` (stage.rs) — four arms: Symbol first topic
  (unchanged); String label + Symbol name; String name (Phoenix family,
  known compromise documented: a future String-label + String-name protocol
  would lift the label — wrong but visible, unlike the silent NULL);
  anything else non-empty warns ("task 0517 monitor" — the warn IS the
  monitored counter, surfacing in CloudWatch like every parser warn) and
  keeps NULL. Empty topic vectors stay silently NULL.
- Unit tests pin all four arms with verbatim production shapes, including
  single-String and String+bytes variants.
- Backfill: in-DB `INSERT … SELECT` per partition (~300-420M rows each,
  quota-safe; the bare NULL filter would scan 10G+ rows) — the exact SQL,
  mirroring the Rust rule, lives in `docs/backfills.md` § "Event-name
  backfill (task 0517)", with its zero-check.

## Rule verified against production (2026-09-02, post-merge of PR #443)

The rule was executed as SQL over the NULL population in three 1M-ledger
windows (~987k rows, ~26% of the population, three eras):

| Window (ledgers) | arm 2 (label+sym) | arm 3 (String name) | unresolved |
| ---------------- | ----------------: | ------------------: | ---------: |
| 63.3-64.3M       |           264,001 |              52,426 |      **0** |
| 58.5-59.5M       |           114,803 |             232,509 |        442 |
| 55-56M           |           128,253 |             195,112 |         23 |

The 465 unresolved rows were inspected individually: their first topic is a
MAP (`{allowee: …}` / `{assignee: …}` — a permissions protocol, 441+153
rows across windows) or a bare ADDRESS — there is no name to lift, so NULL
is the honest value and they land in the monitored arm by design. That
protocol emits nothing in the fresh window, so the live warn stays quiet.
The backfill SQL's `t0='string'` filter excludes them from the insert.
Net: 100% of everything that HAS a name resolves; the residue is nameless
by construction.

## Acceptance Criteria

- [ ] Soroswap pair events carry names (`sync`, `swap`, `deposit`, `withdraw`,
      `skim`)
- [ ] DeFindex vault events carry names
- [ ] emitting protocol label preserved where present
- [ ] historical rows backfilled in-DB; no event with a resolvable name left
      `NULL`
- [ ] residual unresolved count exposed as a monitored metric, not a silence
- [ ] unit tests cover both topic conventions and an unknown third shape
