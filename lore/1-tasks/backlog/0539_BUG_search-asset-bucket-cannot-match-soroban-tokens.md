---
id: '0539'
title: 'BUG: the global search asset bucket cannot match a Soroban-native token, ever'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0370', '0485']
tags: [backend, api, search, assets, soroban, priority-medium, effort-small]
links: []
history:
  - date: '2026-09-04'
    status: backlog
    who: karolkow
    note: >
      NOT a new discovery — this is `0370`'s "Step 3 (optional): global-search
      asset-bucket parity", deferred there because the `contract` bucket
      already returns these tokens by name. 0370 is archived, so the deferred
      step becomes its own task rather than being reopened. Filed after 0485
      unified the match rule across the asset surfaces and the asymmetry stopped
      being defensible: the two surfaces now share ONE rule for the code and
      still disagree about which COLUMNS that rule is applied to. Numbers below
      re-measured on production 2026-09-04.
---

# BUG: the global search asset bucket cannot match a Soroban-native token, ever

## The defect

A Soroban-native (type-3) asset carries an **empty `asset_code`** — its
identity is the contract, and its human-readable name and symbol live in
`soroban_contract_metadata`. The `/v1/search` asset bucket matches the
displayed code and nothing else, and the displayed code of a type-3 row is the
empty string. `position('', needle)` is `0` for every non-empty needle, so the
bucket cannot return one of these assets **for any query at all**.

Measured on production, 2026-09-04:

|                                         |             |
| --------------------------------------- | ----------- |
| type-3 assets                           | **4,413**   |
| of those, with an empty `asset_code`    | 4,413 (all) |
| of those, carrying a symbol in metadata | **3,838**   |
| `/v1/search?q=PIEF` — asset-bucket hits | **0**       |
| `/v1/assets?filter[code]=PIEF` — hits   | **1**       |

`PIEF` is the symbol of the token named `piefox`; the same holds for `POOL`,
`CPAL`, `SMOL` and every other type-3 symbol.

## Why it is not invisible, and why it still matters

The `contract` bucket DOES find these — it matches
`soroban_contract_metadata.name`, so `PIEF` returns one contract hit. That is
why 0370 deferred this: nothing disappears from the product.

What the user gets instead is a **mis-routed** hit. The thing they searched for
is an asset with a page of its own (`/assets/{C…}`), and the search hands them
the contract page. The asset bucket reports zero, so the result count for
"assets" is wrong on a query that plainly names one.

The asymmetry is now between two surfaces that were deliberately unified:

| surface             | matches                            |
| ------------------- | ---------------------------------- |
| `/v1/assets` list   | displayed code **+ name + symbol** |
| `/v1/search` bucket | displayed code **only**            |

Task 0485 gave both the same rule for the CODE (`common::asset_match`), which
makes the remaining difference harder to justify: one rule, applied to a
different column set on each side.

## Approach

Mirror what the list already does (`assets/queries.rs`, `build_list_seek_sql`):
join `soroban_contracts` → `soroban_contract_metadata` and add the name/symbol
arms to the bucket's `WHERE`.

Open questions for whoever picks this up:

- **Ranking.** 0485's tier compares the displayed CODE. A row matched on its
  name has no code to be a tier of and lands in the "matched somewhere" shelf,
  which is probably right — but a token whose symbol IS the needle arguably
  deserves the exact shelf. Decide explicitly; do not let it fall out by
  accident.
- **Cost.** The bucket is currently a single `assets FINAL` scan plus one
  collapsed `soroban_contracts` join. Adding the metadata join puts it on the
  same footing as the list (measured there at ~44 ms → ~78 ms when the
  `soroban_contracts` dedup was on). Measure against the ~330 ms endpoint
  baseline before and after; the search endpoint's p95 is the NFT bucket, so
  there is headroom, but say so with a number.
- **Duplicate hits.** A type-3 token found by symbol will also come back from
  the `contract` bucket, since both read the same metadata name. Two hits for
  one thing, in two groups, is arguably correct (it IS both) — but it should be
  a decision, not a surprise.

## Acceptance criteria

- [ ] `/v1/search?q=PIEF` (and `POOL`, `CPAL`) returns the token in the ASSET
      bucket, routed to its asset page
- [ ] The `/v1/assets` list and the search bucket match the same columns —
      state the rule in one place
- [ ] Ranking decision for name/symbol matches recorded, not implicit
- [ ] Cost measured for the bucket, before and after, against the endpoint
      baseline
- [ ] Docs: `docs/architecture/database-schema/endpoint-queries-clickhouse/22_get_search.sql`

## Not in scope

Relevance ranking itself (task 0485, shipped) and the NFT bucket's collections
redesign (0485, still open).
