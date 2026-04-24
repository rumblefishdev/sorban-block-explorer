---
id: '0165'
title: 'DOCS: Refresh wiki architecture snapshot (Rust backend) post ADR 0027-0037'
type: DOCS
status: active
related_adr:
  [
    '0026',
    '0027',
    '0028',
    '0029',
    '0030',
    '0031',
    '0032',
    '0033',
    '0034',
    '0035',
    '0036',
    '0037',
  ]
related_tasks:
  [
    '0093',
    '0095',
    '0117',
    '0131',
    '0136',
    '0137',
    '0140',
    '0145',
    '0146',
    '0147',
    '0148',
    '0149',
    '0150',
    '0151',
    '0152',
    '0154',
    '0155',
    '0157',
    '0158',
    '0159',
    '0163',
    '0164',
  ]
tags: ['phase-maintenance', 'effort-medium', 'priority-low', 'wiki']
links: []
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Task created. Existing snapshot dated 2026-04-01 predates ADR 0027-0037
      (surrogate IDs, read-time XDR fetch pivot, *_appearances pattern, enum
      smallint, tokens→assets rename). Snapshot describes a model that no
      longer exists.
  - date: '2026-04-24'
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active.'
---

# DOCS: Refresh wiki architecture snapshot (Rust backend) post ADR 0027-0037

## Summary

`lore/3-wiki/project/architecture-snapshot-rust-backend.md` is dated
2026-04-01. Since then ADRs 0026-0037 and ~22 tasks reshaped the system
fundamentally: surrogate BIGINT IDs everywhere, parsed-artifact strategy
abandoned in favour of read-time XDR fetch (ADR 0029), `*_appearances` index
pattern for events/invocations/operations, enum → smallint, tokens renamed
to assets, two new crates. Old snapshot is no longer a drift — it describes
a different system.

## Context

### Drift inventory (2026-04-01 → 2026-04-24)

**New ADRs (0026-0037):**

| ADR  | Shift                                                               |
| ---- | ------------------------------------------------------------------- |
| 0026 | accounts surrogate BIGINT id                                        |
| 0027 | post-surrogate schema + endpoint realizability (**baseline reset**) |
| 0028 | parsed-ledger-artifact v1 shape (superseded by 0029)                |
| 0029 | **abandoned parsed artifacts — read-time XDR fetch**                |
| 0030 | contracts surrogate BIGINT id                                       |
| 0031 | enum columns SMALLINT + Rust enum                                   |
| 0032 | evergreen `docs/architecture/**` maintenance policy                 |
| 0033 | `soroban_events_appearances` read-time detail                       |
| 0034 | `soroban_invocations_appearances` read-time detail                  |
| 0035 | drop `account_balance_history`                                      |
| 0036 | rename tokens → assets                                              |
| 0037 | current schema snapshot (**authoritative schema reference**)        |

**Implementation waves:**

- **Pre-0027 foundation:** 0100-0119 (CI, local dev, 0116 concurrency fix, 0117 backfill-bench)
- **Surrogate ID migration:** 0131, 0136, 0137, 0151, 0152
- **Big-bang schema reset:** 0140 (implement ADR 0027 from scratch)
- **Write/read path pivot:** 0145 → 0146 → 0147 → 0148 → 0149 → 0150
  (postgres backfill-runner → shared parsed-artifact core → live galexie
  Lambda → remove legacy write path → new write path → API XDR fetch read path)
- **Rename/cleanup:** 0154 (tokens→assets), 0155 (ADR audit), 0157-0159,
  0163-0164 (appearances refactors + column drops)

**Repository layout changes:**

- 9 crates now (was fewer): `api`, `indexer`, `xdr-parser`, `db`,
  `db-migrate`, `db-partition-mgmt`, `backfill-bench`, `backfill-runner`,
  `domain`
- Monorepo root has `crates/`, `web/`, `libs/`, `apps/`, `infra/aws-cdk` (TS CDK),
  `scripts/`, `tools/`, `docs/`
- Old snapshot described post-0094/0095 flatten as _target_; now it is
  current state

### Non-drift (still accurate in old snapshot)

- axum / utoipa / sqlx / lambda_http stack choice — unchanged
- cargo-lambda build toolchain — unchanged

## Implementation Plan

### Step 1: Overwrite in place

Overwrite `lore/3-wiki/project/architecture-snapshot-rust-backend.md`.
Git history preserves the 2026-04-01 version. Rationale: wiki CLAUDE.md
mandates _"Living documentation. Current state, not history. Focus on 'what
is' not 'what was'."_ Dated filenames imply historical archive, which is
what git already provides. Filename should describe what the file IS, not
when it was written.

### Step 2: Draft new snapshot

Cover:

1. **Backend stack table** — verify versions against workspace `Cargo.toml`
2. **Workspace layout** — all 9 crates + web + libs + infra/aws-cdk roles
3. **Schema model** — link ADR 0037 as source of truth; highlight:
   - surrogate BIGINT ids (accounts, contracts)
   - `*_appearances` index pattern (events/invocations/operations/transactions)
   - SMALLINT enums (ADR 0031)
   - assets nomenclature (ADR 0036)
4. **Ingestion pipeline** — live galexie Lambda → write path (0149) →
   partition automation (db-partition-mgmt); backfill via `backfill-runner`
5. **Read path** — API fetches XDR on demand per ADR 0029; no parsed
   artifacts stored
6. **Documentation model** — link to `docs/architecture/**` (evergreen per
   ADR 0032); snapshot = photograph, architecture docs = living

### Step 3: Do NOT duplicate `docs/architecture/**`

Snapshot must be a navigable overview + pointers, not a second copy of the
evergreen docs.

### Step 4: Flag frontend-stack drift (out of scope)

`lore/3-wiki/project/frontend-stack.md` also dated — references pending
tasks (0039, 0046, 0047, 0077) that have since moved. Do NOT touch in this
task; scope discipline per lore-framework (one task, one concern). Spawn
separate backlog task with `related_tasks: ["0165"]` if drift confirmed.

## Acceptance Criteria

- [ ] Snapshot reflects ADRs 0026-0037 and repo state at time of writing
- [ ] All 9 crates enumerated with one-line role each
- [ ] ADR 0029 pivot (parsed artifacts → read-time XDR) called out explicitly
- [ ] ADR 0037 cited as authoritative for schema; snapshot does not restate schema
- [ ] `*_appearances` pattern explained once (not per table)
- [ ] No duplication of `docs/architecture/**` content — pointers only
- [ ] Commit overwrites old file in single commit so `git log --follow`
      surfaces 2026-04-01 version cleanly
- [ ] **Docs updated** — N/A — wiki snapshot IS the doc artifact. Underlying
      `docs/architecture/**` is maintained evergreen per ADR 0032 by the tasks
      that drove each change.

## Notes

### Filename convention decision (resolved 2026-04-24)

**Overwrite in place.** Per wiki CLAUDE.md "living doc, current state" rule
and lore-framework guidance that wiki = "what IS" (not "what was").
Git history is the archive; dated filenames would duplicate that role.

### Possible follow-up tasks

- `frontend-stack.md` refresh (separate task — pending 0039/0046/0047/0077
  likely shipped)
- Cadence policy: re-snapshot after every N architectural tasks, or
  quarterly — `/schedule` candidate

### Why this is not just an evergreen-docs update

`docs/architecture/**` tracks the system as it is, file-by-file.
`lore/3-wiki/project/architecture-snapshot-*.md` is a **narrative overview**
useful for onboarding a stateless Claude session or a new human — cross-cuts
what the evergreen docs split across files.
