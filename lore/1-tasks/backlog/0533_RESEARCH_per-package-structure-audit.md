---
id: '0533'
title: 'RESEARCH: per-package structure audit — find the right shape for each package, not one shape for all'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0366']
tags:
  [architecture, frontend, backend, monorepo, priority-medium, effort-medium]
links: []
history:
  - date: '2026-09-02'
    status: backlog
    who: karolkow
    note: >
      Filed after a spot check of `web/src` while shipping 0443 found one
      feature spread across three directories. The check was shallow and
      covered the frontend only; this task is the thorough version, and it
      covers every package because the right shape differs per package.
---

# RESEARCH: per-package structure audit

## Summary

Decide, per package, which organising principle fits it — and say why. The
monorepo currently mixes principles without having chosen any: the frontend is
layer-based at the top and entity-based one level down, while the API crate is
already a clean vertical slice. The goal is not one pattern everywhere; it is a
deliberate, written choice for each package, plus the smallest set of moves
that gets there.

## Context — what the spot check already found

Shallow pass over `web/src` on 2026-09-02 (232 TypeScript files), recorded here
so the audit starts from evidence rather than from scratch:

- **One feature lives in three places.** Accounts: the two screens in `pages/`,
  seven components in `pages/accounts/`, three data hooks in `api/hooks/`.
  Contracts, assets, transactions and pools repeat the pattern.
- **`pages/` holds things that are not pages** — `url.ts`, `cursorParams.ts`,
  `format.ts` sit beside 19 screen components.
- **`pages/detail/` is a shared-primitives folder that is not a page.** Eleven
  files (`SectionCard`, `SummaryRow`, `DataListCard`, four skeletons) used
  across the whole app, filed under "pages".
- **Ad-hoc shared folders appear where pressure built** — `pages/pool-shared/`,
  `pages/transaction-detail/shared/`.
- **`api/hooks/` is a flat pile of 29 hooks**, one per endpoint, ordered by
  nothing.
- **Two counter-examples already in the tree**: `web/src/search/` is a real
  feature folder (components, hooks and logic together) and `libs/ui` is a
  clean design-system boundary. Both are pleasant to work in — evidence that
  the target shape is already understood here, just not applied.

By contrast the backend is in better health and should not be assumed to need
the same treatment:

- `crates/api/src/<entity>/{dto,handlers,queries,mod}.rs` — the same shape in
  eight of ten modules, with justified exceptions (`search/` adds a
  classifier, `contracts/` and `network/` add a cache).
- `domain` (types) and `db-clickhouse` (persistence) are separate crates, so a
  light hexagonal split already exists.
- `xdr-parser` (21.5k lines) is organised flat by XDR concept, which reads
  fine, but `state.rs` is 3415 lines and is the real hotspot — a file problem,
  not a layout problem.
- `backfill-runner` (9.7k lines) is one file per subcommand, which fits a CLI
  tool.
- `indexer` (2k lines) is too small to split.

## Scope

For each package — `web`, `libs/ui`, `libs/api-types`, `crates/api`,
`crates/xdr-parser`, `crates/indexer`, `crates/backfill-runner`,
`crates/enrichment-*`, `crates/db-clickhouse`, `crates/domain`, `infra`,
`infra-hetzner`:

1. Name the principle it follows **today** (layer, feature/vertical slice,
   route, command-per-file, flat-by-concept, none).
2. Name the principle that **fits it**, with the reason tied to how that
   package actually changes — not to fashion.
3. List the smallest set of moves to get there, ranked by payoff, with an
   honest cost. A package whose answer is "leave it alone" is a valid and
   expected outcome; say so explicitly rather than inventing work.

## Acceptance criteria

- [ ] Every package above has a today/target/verdict entry
- [ ] At least one package is explicitly marked "no change needed", or the
      audit explains why none qualified
- [ ] Moves are ranked by payoff and carry a cost estimate; no big-bang
      migration is proposed where incremental moves would do
- [ ] The decision is recorded as an ADR so later sessions inherit the
      reasoning instead of re-litigating it
- [ ] **Docs updated** — `docs/architecture/**` if the audit changes how the
      system is described; `N/A` otherwise, with the reason
- [ ] **API types regenerated** — N/A unless `crates/api/**` moves

## Notes

Two traps this audit must avoid:

- **Doctrine over payoff.** "Feature-based everywhere" is not the goal. The API
  crate is already coherent; moving it to satisfy a pattern name would cost
  review noise and buy nothing.
- **A big-bang move.** The frontend moves are mechanical (imports are
  relative), so they can land one directory at a time. A single 232-file
  commit is unreviewable and will be rejected on sight.
