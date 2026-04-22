---
id: '0155'
title: 'DOCS: audit `docs/architecture/**` against ADR history, bring up to date'
type: DOCS
status: backlog
related_adr: ['0032']
related_tasks: ['0154']
tags: [docs, audit, priority-medium, effort-medium]
links:
  - docs/architecture/technical-design-general-overview.md
  - docs/architecture/backend/backend-overview.md
  - docs/architecture/database-schema/database-schema-overview.md
  - docs/architecture/frontend/frontend-overview.md
  - docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
  - docs/architecture/infrastructure/infrastructure-overview.md
  - docs/architecture/xdr-parsing/xdr-parsing-overview.md
  - lore/2-adrs/
  - lore/1-tasks/backlog/0154_REFACTOR_rename-tokens-to-assets/notes/R-assets-vs-tokens-taxonomy.md
history:
  - date: '2026-04-22'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from ADR 0032 which commits to keeping `docs/architecture/**`
      evergreen. This is the one-shot backward-looking sweep: walk every
      ADR 0022 → 0031, compare to the current state of every file under
      `docs/architecture/**`, and update the docs to reflect reality. After
      this sweep, steady-state maintenance (per-ADR updates) takes over.
---

# DOCS: audit `docs/architecture/**` against ADR history, bring up to date

## Summary

One-shot sweep that brings `docs/architecture/**` in line with the current
state of the system, using the ADR trail as the authoritative source for
"what changed since these docs were written". Established as the catch-up
step for ADR 0032 (evergreen docs policy). After this task, the per-ADR
maintenance process defined in ADR 0032 takes over.

## Context

ADR 0032 establishes that `docs/architecture/**` becomes evergreen going
forward. That policy is only meaningful if the docs are correct _today_.
They currently aren't — the tokens-vs-assets research note documents
concrete drift in `technical-design-general-overview.md` §6.7 alone (four
mismatches on one table), and there's no reason to expect the other files
and sections to be in better shape.

Scope of "the docs":

```
docs/architecture/
├── technical-design-general-overview.md
├── backend/backend-overview.md
├── database-schema/database-schema-overview.md
├── frontend/frontend-overview.md
├── indexing-pipeline/indexing-pipeline-overview.md
├── infrastructure/infrastructure-overview.md
└── xdr-parsing/xdr-parsing-overview.md
```

Scope of "the ADRs that may have caused drift":

- 0022 — schema correction + token metadata enrichment
- 0023 — tokens typed metadata columns
- 0024 — hashes as BYTEA
- 0025 — final schema v1
- 0026 — accounts surrogate (BIGINT id)
- 0027 — post-surrogate schema + endpoint realizability
- 0028 — parsed ledger artifact v1 (later abandoned)
- 0029 — abandon parsed artifacts in favour of read-time XDR fetch
- 0030 — contracts surrogate (BIGINT id)
- 0031 — enum columns SMALLINT + Rust enum

Earlier ADRs (0001–0021) predate the current code generation and are
either process-level, infrastructure-level, or already superseded by
0022+. The auditor should still skim them to confirm no orphaned
decisions were missed, but the deep comparison starts at 0022.

## Implementation Plan

### Step 1: Build the ADR → doc impact matrix

For each ADR 0022 → 0031, identify which doc file(s) should reflect it.
Produce a table in the task worklog, e.g.:

| ADR  | Topic                                 | Primary docs                                       |
| ---- | ------------------------------------- | -------------------------------------------------- |
| 0022 | Schema correction + token metadata    | database-schema-overview, technical-design §6      |
| 0023 | Typed token metadata columns          | database-schema-overview, technical-design §6.7    |
| 0024 | Hashes as BYTEA                       | database-schema-overview, xdr-parsing, backend API |
| 0025 | Final schema v1                       | database-schema-overview, technical-design §6      |
| 0026 | Accounts surrogate                    | database-schema-overview, indexing-pipeline        |
| 0027 | Post-surrogate schema + realizability | database-schema-overview, backend (endpoints)      |
| 0028 | Parsed ledger artifact v1             | indexing-pipeline, infrastructure                  |
| 0029 | Abandon parsed artifacts              | indexing-pipeline, infrastructure, backend         |
| 0030 | Contracts surrogate                   | database-schema-overview, xdr-parsing, indexing    |
| 0031 | SMALLINT enums                        | database-schema-overview, backend                  |

The matrix is the checklist; it drives the sweep and prevents missed
updates.

### Step 2: Per-file reconciliation pass

For each file under `docs/architecture/**`, walk the matrix entries that
touch it. For each, compare the doc's description to the current code /
migration state. When they differ, rewrite the section to match current
state; link the relevant ADR(s) for the "why".

Where the research note
[task 0154's `R-assets-vs-tokens-taxonomy.md`](../0154_REFACTOR_rename-tokens-to-assets/notes/R-assets-vs-tokens-taxonomy.md)
§5.2 already calls out specific drift (the four `tokens`-section
mismatches), those are a pre-filled starting checklist — but **do not
rely on the note being exhaustive**; it only covers the tokens section
because that's where its question pointed. Every other section needs a
fresh comparison pass.

### Step 3: Coordinate with task 0154

Task 0154 (tokens → assets rename) touches the same schema sections. If
0155 lands first, 0154's doc update shrinks to the rename delta. If
0154 lands first, 0155 inherits a clean tokens/assets baseline and
focuses on everything else. Either order works; just don't run them in
parallel on the same files without coordination.

### Step 4: Formalise the per-ADR maintenance process

As part of this task's deliverable, update:

- `lore/2-adrs/_template.md` — add a "Docs updated" line to the
  acceptance criteria section (or equivalent).
- `lore/1-tasks/_template.md` — same, for tasks that implement an ADR.
- `CLAUDE.md` (root) or the ADR-level `CLAUDE.md` — capture the rule so
  future Claude sessions enforce it.

This closes the loop: after 0155 lands, any future ADR PR that forgets
to update docs will fail the template's own checklist.

## Acceptance Criteria

- [ ] Worklog contains the ADR → doc impact matrix (Step 1 output).
- [ ] Every file under `docs/architecture/**` has a completed
      reconciliation pass, documented in the worklog (one entry per file
      with "no changes", "minor sync", or "rewritten — diff summary").
- [ ] Concrete drift points from the research note §5.2 are all
      addressed in `technical-design-general-overview.md` §6.7 (or
      wherever the `tokens`/`assets` table is described post-rename).
- [ ] Each rewritten section that reflects an ADR-driven change links
      to the relevant ADR(s) for context.
- [ ] `lore/2-adrs/_template.md` and `lore/1-tasks/_template.md` updated
      with a "Docs updated" checklist entry.
- [ ] Root `CLAUDE.md` (or the relevant sub-`CLAUDE.md`) documents the
      per-ADR maintenance rule defined by ADR 0032.
- [ ] PR review pass — a second team member confirms at least one
      per-file reconciliation against the current code, to catch
      blind-spot errors.
- [ ] Markdown lint (if the project has one) passes on all touched
      files.

## Risks

- **Scope creep.** "Update the docs" is open-ended. Mitigation: the
  matrix in Step 1 is the contract. Changes outside it are out of
  scope and get spawned as follow-up tasks.
- **Stale at merge.** If the audit takes a week and new ADRs land
  during it, the doc changes can go stale before the PR merges.
  Mitigation: keep the task short (1–3 days), avoid opening it during
  a stretch with other ADRs in flight, or rebase once at the end.
- **Over-rewriting.** Temptation to restructure sections that are
  merely awkwardly worded but not wrong. Mitigation: keep the sweep
  mechanical — fix what's wrong, leave style alone.
- **Code-drift, not ADR-drift.** Some docs may be wrong due to changes
  that didn't get an ADR (informal fixes). Mitigation: during the
  comparison pass, note any such cases; if material, spawn a backlog
  task to write a retrospective ADR.

## Notes

- Preserve history when rewriting sections. Prefer `git mv` + edit over
  deletion + creation so `git blame` still reaches the original author.
- Do not edit ADRs themselves. Per ADR 0032 §4, ADRs stay as immutable
  historical records; this task only touches `docs/architecture/**`
  and the two templates + root CLAUDE.md called out above.
- Research note §6.6 (XLM-SAC linkage gap) is _not_ in scope here —
  it's a data-model question, not a doc-drift question. If the doc
  describes XLM handling, the audit may flag it as "current behavior
  has a known limitation, noted in note §6.6" and link the note, but
  it does not try to resolve the underlying gap.
