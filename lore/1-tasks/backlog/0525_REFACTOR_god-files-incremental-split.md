---
id: '0525'
title: 'God files — incremental split by topic, tests always extracted'
type: REFACTOR
status: backlog
related_adr: []
related_tasks: ['0374']
tags: ['phase-future', 'effort-medium', 'priority-low', 'code-health']
links: []
history:
  - date: '2026-09-02'
    status: backlog
    who: karolkow
    note: >
      Absorbed 0526, which was the same task filed twice on the same day from
      the same 0374 simplify round — same subject, same parent, both in
      backlog. 0526's file is retired rather than left as a second task on one
      subject (the 0470/0471 precedent). Everything it carried that this one
      did not is folded in below: `contracts/queries.rs` in the table, the
      per-PR method, and the pure-move commit convention. The repo `CLAUDE.md`
      carried the same section twice for the same reason, one half pointing at
      each id; deduplicated in this commit and pointed at 0525.
  - date: 2026-08-31
    status: backlog
    who: claude
    note: "Spawned from 0374's simplify pass (requester karolkow). CLAUDE.md rule landed the same day; this task is the incremental cleanup of the existing stock."
---

# God Files — Incremental Split by Topic

## Summary

Several production modules have grown into god files — everything of one
layer in one file, tests inline. Shrink the existing stock INCREMENTALLY
(when a task touches the file), following the File Size & Test Placement
rule in the repo `CLAUDE.md`. Never a big-bang refactor.

## Context

Measured 2026-08-31 (production lines, tests already excluded where split):

| File                                                          |                                        Lines |
| ------------------------------------------------------------- | -------------------------------------------: |
| `crates/xdr-parser/src/state.rs`                              |                                        3,427 |
| `crates/db-clickhouse/src/persist/tests_cross.rs`             | 3,301 (test file — split by TOPIC, not size) |
| `crates/db-clickhouse/src/persist/stage.rs`                   |                                        3,093 |
| `crates/api/src/liquidity_pools/queries.rs`                   |      3,140 (tests already extracted in 0374) |
| `crates/xdr-parser/src/operation.rs`                          |                                        1,633 |
| `crates/xdr-parser/src/invocation.rs`                         |                                        1,580 |
| `crates/enrichment-shared/src/nft_token_uri/client.rs`        |                                        1,572 |
| `crates/api/src/assets/queries.rs`                            |                                        1,362 |
| `crates/api/src/liquidity_pools/handlers.rs`                  |             ~1,240 (tests extracted in 0374) |
| `crates/api/src/contracts/queries.rs`                         |                                        1,245 |
| `web/src/pages/transaction-detail/op-card/ExecutionTrace.tsx` |                                        1,000 |

The TS side is healthier (max 1k, tests in `*.test.tsx` siblings) — the rule
mostly binds Rust.

Precedent: 0079 split `domain` types into modules; 0374 extracted every
inline test module of `crates/api/liquidity_pools` into `*_tests.rs`
siblings and moved the share-token oracle out of `pool_router.rs` into
`tests/`.

## Implementation

- The rule itself lives in the repo `CLAUDE.md` ("File Size & Test
  Placement") — this task is only the EXISTING stock.
- Ratchet, not big bang: whenever a task touches a listed file, that task
  extracts at least the inline tests, and ideally splits one coherent topic
  out (e.g. `stage.rs` → per-table staging modules; `state.rs` → per-entry-
  kind modules; `queries.rs` → per-endpoint query modules).
- Verification-only code (oracles, corpus tests) moves to the crate's
  `tests/` directory, out of the production module — the `pool_router.rs`
  share-token oracle move is the pattern.
- A split is its own `refactor(...)` commit: pure moves, zero behaviour
  change, so the review is `git diff --color-moved`. Never fold a split into
  the feature commit that happened to touch the file.
- Update the table above as files shrink; close the task when nothing
  production exceeds the CLAUDE.md limit.

### Open question, first hit 2026-09-02 (task 0485)

`*_tests.rs` does not exist ANYWHERE in this repo yet — 0374 extracted the LP
module's tests, but that work sits on an unmerged branch, so on `develop` the
convention is still unwritten. 0485 touched `assets/queries.rs` (1,422) and
`search/queries.rs` (1,167) and deliberately did NOT extract: neither file
would have come under 800 afterwards, so the only gain would have been
establishing a repo-wide file convention inside a 46-line bugfix. Decided by
karolkow. If the convention is to be set, it deserves its own PR where the
move IS the whole diff.

## Acceptance Criteria

- [ ] every file listed above is under the CLAUDE.md production-module limit
      or is a test file split by topic
- [ ] no production module holds an inline `#[cfg(test)]` module in the
      crates touched along the way
- [ ] splits happened incrementally inside feature/fix tasks (no standalone
      big-bang refactor PR)
