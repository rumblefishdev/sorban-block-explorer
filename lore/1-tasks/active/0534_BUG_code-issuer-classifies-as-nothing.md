---
id: '0534'
title: 'BUG: the canonical CODE:ISSUER classifies as nothing, so the most precise search returns a blank page'
type: BUG
status: active
related_adr: []
related_tasks: ['0485', '0331', '0470']
tags: [backend, api, search, assets, priority-medium, effort-small]
links:
  - 'https://github.com/rumblefishdev/soroban-block-explorer/issues/368'
history:
  - date: 2026-09-03
    status: backlog
    who: stkrolikiewicz
    note: 'Task created. Both defects reproduced against production ClickHouse.'
  - date: 2026-09-03
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active. Plan needs no research phase — both defects are
      reproduced and the ranking fix is already verified against production.'
  - date: 2026-09-03
    status: active
    who: stkrolikiewicz
    note: >
      SCOPE CUT. Opened without finding 0485 first, and its ranking half
      duplicated work already open as PR #445 — worse, at that: exact-vs-rest
      instead of the exact > prefix > substring tier, and no trailing PK
      columns, so ties on holder_count stayed non-deterministic, which is half
      the defect 0485 names. That half is dropped here and left to 0485; the
      measurements taken while building it are recorded below for whoever picks
      that work up. What remains is the one defect 0485 does not cover:
      CODE:ISSUER classifying as nothing.
---

# BUG: the canonical `CODE:ISSUER` classifies as nothing

## Summary

`USDT0:GATISXX6BZ6NC7IKQBY37CJD4SOZL3CYZJWXEDG6JVIY4WBS6KXJHN6Q` is the
canonical asset form in the SDKs and SEPs, and it is the most precise thing a
user can type. It returns a blank page.

Together with [[0485]] this makes two of the five ways to name one asset dead
ends, from opposite directions:

| input                              | today                      |
| ---------------------------------- | -------------------------- |
| `USDT0`                            | impostors first (0485)     |
| `USDT0:GATISXX…` (SEP / SDK form)  | **empty page** (this task) |
| `USDT0-GATISXX…` (our route token) | **empty page** (this task) |

## Context

[`classify`](../../../crates/api/src/search/classifier.rs) returns
`Classified::default()` — not 64-hex, not an L-strkey, does not begin with
`G`/`C` — so both derived inputs stay `None`. Then:

- the asset arm runs a 60+ character needle against `asset_code` (≤12
  characters): **provably zero rows**;
- the account arm never fires, because the string does not start with `G`.

The vaguer the query, the more it returns.

## Implementation

A `code_issuer` classifier mode splitting on the **last** `:` or `-`. The split
is unambiguous: a Stellar asset code is `alphanum4`/`alphanum12` and a G-StrKey
is base32, so neither separator can occur inside either half. `-` is accepted
alongside `:` because that is the shape our own `/assets/:id` routes emit, so
users paste it back.

The issuer is **decoded in full**, not shape-matched — `from_string` checks the
CRC — so a typo falls back to the ordinary substring search instead of answering
with a confidently empty page.

`search_assets` then takes an equality arm: `asset_code = ? AND issuer = ?`. It
needs no ranking (a qualified pair names one row) and resolves the issuer
through the `accounts` `ORDER BY account_id` key as a point seek, never the
~23M-row hash join that OOMs (Code 241).

## Acceptance Criteria

- [x] `USDT0:GATISXX…` and `USDT0-GATISXX…` both resolve to exactly one asset
      hit — both separators covered by a live-CH smoke, and the exact arm
      returns a single row on production data
- [x] A `CODE:ISSUER` whose issuer fails checksum degrades to today's
      behaviour, not a 500 — `code_issuer_rejects_a_bad_issuer_checksum`;
      classification falls through to the substring arm, so it stays a 200
- [x] A bare StrKey still classifies as a prefix — the new arm must not shadow
      the account/contract lookup (`a_bare_strkey_is_still_a_prefix_not_a_code_issuer`)
- [x] **Docs updated** — `22_get_search.sql` carries the new `code_issuer`
      classifier mode and the exact-lookup arm. Other `docs/architecture/**`
      N/A: no new table, endpoint or pipeline step.
- [x] **API types regenerated** — run; the diff is **empty**, as expected.
      `Classified` is internal and no response shape moved.

## Notes

Two fixture traps, both worth knowing before writing a similar test:

- **`FULL_G` in the existing classifier tests is shape-only, not CRC-valid.**
  Fine for the prefix arm, which never decodes; this arm does, so it needed a
  real key. The local seed data had the same problem and the test caught it —
  the first run returned zero hits until the keys were regenerated.
- **`PublicKey::to_string()` is an inherent method returning
  `heapless::String`**, not `std::String` — the trap `common::strkey` already
  documents. `format!` avoids the double-`to_string()` dance.

## For 0485 — measurements taken while the ranking half was still in scope

Recorded here rather than edited into 0485, which has an open PR (#445) and
would have conflicted. Everything below is production, 2026-09-03.

**The join is not free.** Same shape as the reverted commit's numbers, and they
agree with 0485's "+43 ms, below the noise floor" reading:

| form                                       | rows      | read      | ms  |
| ------------------------------------------ | --------- | --------- | --- |
| substring arm, no join                     | 845,458   | 25.18 MiB | 91  |
| + `balance_aggregates` join and `ORDER BY` | 1,456,830 | 37.75 MiB | 138 |

Two cheaper shapes measured and rejected, so they need not be re-tested:

- **narrowing the join's right side** to `holder_count > 0` — 354k of 447k rows
  qualify, so it saves nothing and the extra filter measured _worse_, 144 ms;
- **ranking in Rust** (page, then seek `balance_aggregates` by id — the seek
  itself is cheap, 7 ms) — needs the whole match set to rank correctly, and a
  one-letter needle matches **181,388** assets. Capping the page reintroduces
  the arbitrary cut, an order of magnitude higher.

**Bucket latencies.** Matching a bucket by table name sweeps in its LIST
endpoint — `/v1/liquidity-pools` reads 11.6 M rows / 460 MiB per call against a
75k-row table, which reads as a 527 ms "pool bucket" that does not exist.
Matched on each bucket's own predicate, `api_reader` only, 7 days:

| bucket      | n   | p50 | p95 | max | max read   |
| ----------- | --- | --- | --- | --- | ---------- |
| nft         | 112 | 163 | 189 | 716 | 13.03 MiB  |
| asset       | 112 | 114 | 136 | 746 | 25.46 MiB  |
| pool        | 62  | 74  | 93  | 335 | 4.01 MiB   |
| contract    | 85  | 10  | 14  | 48  | 366.64 KiB |
| transaction | 29  | 9   | 11  | 11  | 11.04 MiB  |
| account     | 27  | 4   | 10  | 13  | 4.50 MiB   |

`max read` is the check that the filter is honest: `pool` peaks at 4.01 MiB
against a 3.97 MiB table. The buckets run concurrently under
`tokio::try_join!`, so the endpoint's p95 is `nft`, and ranking the asset
bucket (136 → ~200 ms) moves it by roughly 6%.

**Bucket 3 (NFTs), from the perf side.** The redesign to `collection_name` is
the right call and this does not change it, but two mechanical defects sit in
`search_nfts` today and would survive a rewrite that keeps the CTE shape:

- **`page` is evaluated twice** — referenced by `sc` (`IN (SELECT … FROM page)`)
  and again by the final `FROM page p`, and CH does not materialise CTEs.
  Measured: the full query reads 116,494 rows, `page` alone reads 58,247,
  exactly double. Two round-trips in Rust instead: 155 ms → 76 ms (73 + 3),
  memory 247 MiB → 114 MiB. Same trap the asset bucket's own comment documents
  from 0420, two functions above.
- **`enr` aggregates the whole table before filtering** — `argMax` over all of
  `nft_enrichment` costs 100+ MiB for a 984 KiB table. Filtering before the
  `GROUP BY`: 95 ms → 63 ms on a needle with 10,046 matches. (A needle with
  zero hits showed 7×; that figure was not representative.) Watch the alias:
  `argMax(name, version) AS name` makes `WHERE` bind the aggregate and CH
  rejects it with `ILLEGAL_AGGREGATION`.
