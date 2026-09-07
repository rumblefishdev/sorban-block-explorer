---
id: '0522'
title: 'REFACTOR: one asset-identity read path (surrogate → display identity)'
type: REFACTOR
status: backlog
related_adr: ['0051']
related_tasks: ['0374', '0344', '0345', '0496', '0231']
tags: [backend, clickhouse, api, assets, read-path, priority-low, effort-medium]
links: []
history:
  - date: '2026-08-28'
    status: backlog
    who: karolkow
    note: >
      Filed from 0374 step 13: resolving a display identity from a surrogate
      touches assets + asset_sac (+ enrichment for names/icons), and every
      consumer used to hand-assemble the joins. TRIGGER-GATED — see below.
---

# REFACTOR: one asset-identity read path

## The smell, named

Getting "what do I call this asset" from an `Int64` surrogate takes three
tables. The write-side split is measured necessity, not mess — `asset_sac`
and `asset_enrichment` exist because the versionless-RMT `assets` rewrite
clobbered mutable columns (ADR 0051 storage correction; task 0231) — but the
read side made every consumer re-derive the same joins.

## What exists already — do not rebuild it

The house pattern is the **0344/0345 id-IN resolver**: point-seek a dimension
by an id list instead of hash-joining the whole table. Two instances live:

- `resolve_accounts` (accounts dimension, 0345)
- `resolve_leg_assets` (asset identity for pool legs, 0374 step 13 —
  `asset_sac` arm + bespoke type-3 arm, decimals included)

## The closure this task builds (when triggered)

One canonical `asset_identities` read surface — a view or dictionary keyed by
surrogate: `id → (family, code, issuer_id, contract strkey, decimals, name,
icon)` — assembled ONCE from assets + asset_sac + asset_enrichment +
soroban_contract_metadata, with the AMT/RMT read protocols (GROUP BY + max /
argMax by version) applied in exactly one place. Existing resolvers re-point
at it; new consumers `dictGet`/join it.

## Trigger — depth-first gate (Karol, 2026-08-28)

**Do not start until a THIRD consumer needs richer identity fields** than a
current resolver serves. Two instances are a pattern; a third is the signal
to consolidate. Until then this file is the address where the smell is
recorded, so nobody re-litigates it from scratch.

## Acceptance Criteria

- [ ] one read surface serves every surrogate→identity consumer
- [ ] AMT/RMT read protocols live in exactly one definition
- [ ] `resolve_accounts` / `resolve_leg_assets` re-pointed or retired
- [ ] read cost measured before/after on the union pool list and account page
