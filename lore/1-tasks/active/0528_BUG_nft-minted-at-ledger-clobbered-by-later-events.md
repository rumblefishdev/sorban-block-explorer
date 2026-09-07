---
id: '0528'
title: 'nfts.minted_at_ledger clobbered by any post-mint event — serve it from nft_ownership'
type: BUG
status: active
related_adr: ['0043', '0044']
related_tasks: ['0051', '0217', '0529']
tags: ['nft', 'clickhouse', 'api', 'data-integrity', 'effort-small']
links: []
history:
  - date: '2026-09-01'
    status: active
    who: karolkow
    note: >
      Filed from a prod measurement. 621 of 13 915 NFT tokens (4.5%) serve a
      NULL `minted_at_ledger` on `GET /v1/nfts` and `GET /v1/nfts/:id`, so the
      detail page renders an empty "Minted at ledger" and the list sorts them
      as if minted at ledger 0. Every one of the 621 has its Mint row intact in
      `nft_ownership` — nothing was lost from the chain, only the denormalised
      copy. Root cause is the RMT replace semantics, not the parser.
  - date: '2026-09-01'
    status: active
    who: karolkow
    note: >
      Read-path fix implemented and committed (`8089d2b2` on
      `fix/0528_nft-minted-at-ledger-from-ownership`, rebased onto develop).
      Verified against prod CH: 623 clobbered tokens (up from 621 at filing, the
      ~30/day rate showing) all resolve, and 13 292/13 292 pre-existing values
      are byte-identical — the derivation is a strict superset. Two CH-gated
      tests added, both confirmed to fail on the pre-fix code, not merely to
      pass on the new. One of them caught a real defect on first run: the
      un-wrapped `min()` returns a non-Nullable Int64 and a LEFT JOIN miss fills
      DEFAULT 0, which would have 500'd the endpoint and rendered a mint-less
      token as "ledger 0" — fixed with `nullIf(_, 0)`.
      End-to-end run against prod CH consciously NOT done (see the AC): the
      certs are agent-blocked and the two halves are separately proved. Awaiting
      merge and deploy; not archived.
---

# nfts.minted_at_ledger clobbered by any post-mint event

## Summary

`nfts` is `ReplacingMergeTree(current_owner_ledger)` with one row per
`(contract_id, token_id)`. A transfer or burn arriving in a later ingest batch
carries `minted_at_ledger = NULL` (the indexer only sees the current batch, and
the mint may be weeks behind), and RMT replaces the **whole row** by the higher
version — so the mint ledger is erased.

The value is never lost from the chain: `nft_ownership` is append-only and
holds the Mint row for all affected tokens. Fix is to stop reading the
denormalised copy and derive the value from `nft_ownership` at query time.
Dropping the now-unread column is deliberately **out of scope** — see 0529.

## Context

### Measured on prod (2026-09-01)

| Token history          | Tokens with NULL `minted_at_ledger` |
| ---------------------- | ----------------------------------- |
| mint + burn            | 600                                 |
| mint + transfer        | 19                                  |
| mint + transfer + burn | 2                                   |
| **total**              | **621 of 13 915 (4.5%)**            |

- All 621 have an `event_type = 0` (Mint) row in `nft_ownership` → 100 %
  recoverable, no re-parse and no backfill needed.
- `nft_ownership` holds 23 092 rows over 13 915 tokens, of which **13 915 are
  Mint rows** — exactly one mint per token, so the read-side aggregate is
  unambiguous and cheap.
- Ongoing corruption rate: **~30 tokens/day** (25–31 burns/day measured over
  the preceding 10 days, plus occasional transfers). A one-shot data repair
  regresses within days.

### Why the existing repair is not the answer

`backfill-runner repair-tier1` already computes exactly the right value
(`rebuild_nfts`, `MIN(ledger_sequence) … WHERE event_type = 0`) and writes it
back into `nfts`. That is a point-in-time correction: it fixes today's 621 and
is undone by the next burn. It also finalises through `EXCHANGE TABLES`
(`ch_staging::finalize`), so it requires the indexer stopped.

Deriving the value at read time makes both the repair run and the stop
unnecessary — there is no stored copy left to go stale.

### Prior art — this was seen before, under a different engine

The 2026-04-10 pipeline audit filed this as **F17 — "NFT minted_at_ledger
Immutable After First Insert"** and rated it MEDIUM / "acceptable for
explorer". That assessment was correct **for Postgres**: the column was
INSERT-only and never in the `UPDATE SET`, so a stale value survived. The
ClickHouse migration inverted the failure mode — RMT replaces the row wholesale,
so "stale but present" became "erased". The audit's proposed mitigation
("post-backfill correction query") is what became `repair-tier1`, and it
carries the same point-in-time limitation described above.

### Where the writer is correct already

In-batch merging in `persist::stage` does the right thing — it folds
`minted_at_ledger` with an explicit `min()` when several events for one token
land in the same batch. The defect is strictly cross-batch, where the merge
policy is the storage engine's and not ours.

## Implementation Plan

### Step 1 — derive the value in the NFT list query

`crates/api/src/nfts/queries.rs`, `fetch_page`. Add a `mint` CTE shaped exactly
like the existing `enr` CTE, and `LEFT JOIN` it:

```sql
mint AS (
    SELECT contract_id, token_id, min(ledger_sequence) AS minted_at_ledger
    FROM nft_ownership
    WHERE event_type = 0
    GROUP BY contract_id, token_id
)
```

`event_type = 0` is `domain::enums::NftEventType::Mint`. The filter must stay
explicit — "earliest ownership row" would be wrong, because a transfer or burn
replayed at an earlier ledger would yield a non-mint ledger. This is the same
reasoning already written down in `repair_tier1::rebuild_nfts`.

### Step 2 — move the sort key and the cursor onto the derived value

`ORDER BY ifNull(n.minted_at_ledger, 0)` and the keyset predicate both key on
this column, and `NftListCursor.minted_at_ledger` encodes it. All three must
read the CTE value, not `n.minted_at_ledger`, or pagination will split across
two different orderings and silently skip or repeat rows.

### Step 3 — same derivation on the detail endpoint

`fetch_by_composite` + `handlers::get_nft`. Detail serves `Option<i64>`; it must
come from the CTE.

### Step 4 — regression test

One test that fails on today's code: a token with a Mint row and a later Burn
row in `nft_ownership`, whose `nfts` row carries `minted_at_ledger = NULL`,
must still serve the mint ledger.

## Acceptance Criteria

- [x] `GET /v1/nfts/:id` serves the mint ledger for a token whose `nfts` row has
      `minted_at_ledger = NULL` but whose `nft_ownership` Mint row is present
- [x] `GET /v1/nfts` serves the same value, and sorts and paginates on it
- [x] Keyset pagination stays total — no skipped or repeated rows across pages
      when the derived value replaces the stored one
- [x] Regression test covering the mint-then-burn shape
- [x] Verified against prod ClickHouse: 623 tokens (621 at filing) serve a
      non-NULL mint ledger with no data migration, no `repair-tier1` run and no
      indexer stop; 13 292/13 292 pre-existing values unchanged
- [x] **Docs updated** — `docs/architecture/database-schema/endpoint-queries-clickhouse/15_get_nfts_list.sql`
      carries the new CTE and the amended cursor note. (That file also predates
      the current cursor shape, which is already
      `(minted_at_ledger, contract_id, token_id)` and not the documented
      `(contract_id, token_id)`; correcting that drift belongs to this task
      because this task is what settles the final shape.)
- [x] **API types regenerated** — ran `npx nx run @rumblefish/api-types:generate`;
      **empty diff confirmed**, not assumed. The `minted_at_ledger` field keeps
      its name, type and nullability; only its source changed.
- [x] End-to-end check — **covered in parts, NOT as one run.** Deliberate call,
      taken 2026-09-01 with the residual gap stated below rather than hidden.
      The two halves are each proved against the shapes that matter:
      the SQL on real production data (623 corrected, 13 292 unchanged, via
      `chq`), and the Rust decode + query on the exact mint-then-burn shape (the
      CH-gated tests, both confirmed to fail on the pre-fix code).
      **Residual gap:** nothing has run database → handler → JSON in one pass on
      real data, so a fault in the serving layer alone would not have been seen.
      Judged small — the wire shape is unchanged (empty codegen diff) and the
      RowBinary decode is the layer the tests exercise. `api --bin local`
      requires the mTLS bundle and `~/.certs` is blocked to the agent by an
      active permission rule, so closing this properly is an operator step:
      stage the certs, run the binary, and curl a clobbered token.

## Implementation Notes

Branch `fix/0528_nft-minted-at-ledger-from-ownership`. Three files, +185/−15.

- `crates/api/src/nfts/queries.rs` — `mint` CTE in `fetch_list` (mirrors the
  existing `enr` CTE), a scoped `mi` sub-select in `fetch_by_composite`, and the
  sort key / keyset predicate / cursor payload all moved onto the derived value.
- Both `docs/architecture/database-schema/endpoint-queries-clickhouse/`
  15 + 16 updated, plus an explicit DRIFT NOTICE — see Design Decisions.
- Two CH-gated tests added.

### Verification

Against **prod ClickHouse** (read-only, via `chq`):

- 623 tokens (up from 621 — two more clobbered in the hours between filing and
  fixing, consistent with the ~30/day rate) → **0 still NULL** under the
  derivation.
- Where a stored value exists: **13 292 / 13 292 identical**, 0 changed. The fix
  is a strict superset — it repairs 623 and moves nothing else.
- Spot check, "Talk" token 83305 (alive, 4 transfers, never burned):
  stored `NULL` → derived `58946561`, matching its Mint row.

Against a **local ClickHouse** with a seeded 7-token fixture (3 healthy, 4
clobbered, mint ledgers interleaved so an ordering mismatch cannot cancel out):

- `cargo test -p api --lib nfts::` → 4 passed, 0 failed
- `cargo fmt --check` clean, `cargo clippy --all-targets` 0 warnings
- `npx nx run @rumblefish/api-types:generate` → **empty diff**, confirming the
  wire shape is unchanged (checked, not assumed)
- Both new tests were confirmed to FAIL on the pre-fix behaviour, not merely to
  pass on the new one: the detail test against the stored column, and the
  pagination test with the keyset pointed back at `n.minted_at_ledger`
  (`mint-ledger order broke across the page boundary: 140 after 130`).

Not covered: an end-to-end run of `api --bin local` against prod CH. That needs
the mTLS certs staged, which is an operator step.

## Issues Encountered

- **The first version of the fix would have 500'd the live endpoint**, caught by
  the new test on its first run:
  `SchemaMismatch("… ClickHouse type Int64 as Option<T> which is not compatible")`.
  `min(ledger_sequence)` over a non-Nullable column returns a **non-Nullable**
  `Int64`, and — because `api_reader` runs `readonly = 1` and cannot set
  `join_use_nulls = 1` — a LEFT JOIN miss fills the type DEFAULT `0` instead of
  NULL. So the un-wrapped derivation both failed the RowBinary decode and would
  have rendered a mint-less token as "ledger 0" rather than blank.
  Fixed with `nullIf(_, 0)`, which is the idiom this module already documents
  for exactly this ("maps a JOIN miss / sentinel to `None`"). Ledger sequences
  start at 1, so `0` is an unambiguous sentinel. This is the inverse of the
  nullable-aggregate trap from 0324 — same lesson, opposite direction.

- **The endpoint-query docs were already drifted** well beyond this task: they
  still show `collection_name` / `name` / `media_url` read from `nfts` (vestigial
  since 0231), a `(contract_id, token_id)` cursor (superseded), and
  whole-dimension FINAL joins (replaced in 0355).

- The 4 unrelated failures seen when running the whole `api` suite against the
  fixture DB (`liquidity_pools` / `search` asset-code smokes) are missing pool
  data in that seeded database, not regressions — the same suite is 259 + 286
  green with `CH_URL` unset.

## Design Decisions

### From Plan

1. **Derive at read time rather than repair the stored value.** A one-shot
   `repair-tier1` run regresses at ~30 tokens/day and needs the indexer stopped;
   deriving removes the stored copy from the read path entirely, so the 623 are
   correct immediately and permanently, with no migration and no ops window.

2. **Keep the column, drop it separately (0529).** Removing it needs a prod
   `ALTER` whose deploy ordering is the actual hazard (0310). Bundling it here
   would have put an ops-window change behind a pure read-path fix.

### Emerged

3. **`nullIf(_, 0)` on the derived value** — not in the plan; forced by the
   non-Nullable aggregate + no `join_use_nulls`. See Issues Encountered.

4. **DRIFT NOTICE instead of a full doc reconciliation.** Reconciling E15/E16
   with the code they describe is several tasks' worth of unrelated changes.
   Half-correcting them would leave a document that reads as current while still
   being wrong, so the corrected mint-ledger source is accompanied by an explicit
   banner naming what else no longer matches and pointing at the authoritative
   source. Chose this over both "fix everything" (scope creep) and "fix only my
   line" (silently misleading).

5. **Pagination totality test added beyond the AC's single regression test.**
   The mint ledger is referenced separately by the ORDER BY, the keyset predicate
   and the cursor payload; a single-page test cannot see them disagree. The added
   test walks the whole list in 2-row pages and asserts each token appears
   exactly once in non-increasing order.

6. **Tests live in `crates/api` and therefore do NOT run in CI.** CI starts a
   ClickHouse but runs `cargo test -p db-clickhouse -p backfill-runner` only, and
   does not export `CH_URL` to the api crate — so both new tests skip cleanly
   there. They are real guards locally and against prod CH, not CI gates. Wiring
   the api crate's CH-gated tests into CI is a follow-up worth its own task; 0406
   covered only the other two crates.

## Notes

- `nfts.minted_at_ledger` stays in the schema, unread. That is intentional —
  the same status `name`, `media_url` and `collection_name` already hold in that
  table, which the schema comment calls out as vestigial. Removing it is 0529.
- Nothing here touches the indexer or the parser. The write path may keep
  clobbering the column; once nothing reads it, the clobbering is inert.

## Future Work

- **0529** — drop the vestigial `nfts.minted_at_ledger` column.
