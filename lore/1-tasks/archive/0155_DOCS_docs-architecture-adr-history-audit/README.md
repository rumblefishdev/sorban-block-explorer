---
id: '0155'
title: 'DOCS: audit `docs/architecture/**` against full ADR history, bring up to date'
type: DOCS
status: completed
related_adr:
  [
    '0001',
    '0002',
    '0004',
    '0005',
    '0006',
    '0007',
    '0008',
    '0010',
    '0019',
    '0020',
    '0021',
    '0022',
    '0023',
    '0024',
    '0025',
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
  ]
related_tasks: ['0139', '0140', '0154', '0159']
tags: [docs, audit, priority-medium, effort-medium]
links:
  - docs/architecture/technical-design-general-overview.md
  - docs/architecture/backend/backend-overview.md
  - docs/architecture/database-schema/database-schema-overview.md
  - docs/architecture/frontend/frontend-overview.md
  - docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
  - docs/architecture/infrastructure/infrastructure-overview.md
  - docs/architecture/xdr-parsing/xdr-parsing-overview.md
  - docs/database-audit-first-implementation.md
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
  - date: '2026-04-22'
    status: backlog
    who: stkrolikiewicz
    note: >
      Partial coverage landed in 0139: `docs/architecture/database-schema/database-schema-overview.md`
      (partition strategy lines 189/202/513) and `docs/database-audit-first-implementation.md`
      (lines 130, 136-140) corrected for ADR 0027 operations partitioning.
      Remaining hot spots for this task: `docs/architecture/technical-design-general-overview.md`
      (16 stale schema hits per grep), plus rest of `docs/architecture/**`
      for ADR 0030 contracts BIGINT surrogate, ADR 0031 enum SMALLINT,
      and lingering operations_pN references. Should land AFTER 0154
      (tokens→assets rename) to avoid merge churn on shared files.
  - date: '2026-04-23'
    status: active
    who: karolkow
    note: >
      Task activated and assigned.
  - date: '2026-04-23'
    status: active
    who: karolkow
    note: >
      Steps 1-4 executed in one pass. Matrix landed at
      `notes/G-adr-doc-matrix.md` covering ADRs 0022-0031 vs every
      `docs/architecture/**` file + `docs/database-audit-first-implementation.md`.
      Per-file reconciliation tracked in `notes/worklog.md`: 5 files rewritten
      (database-schema-overview, technical-design-general-overview, xdr-parsing-overview,
      indexing-pipeline-overview, infrastructure-overview), 1 minor-synced
      (backend-overview), 1 no-change (frontend-overview), 1 stale-noticed with
      preserved-snapshot treatment (database-audit-first-implementation).
      Templates updated
      (`lore/2-adrs/_template.md` delivery checklist; `lore/1-tasks/_template.md`
      acceptance-criteria entry). Root `CLAUDE.md` carries the ADR 0032
      evergreen rule.
  - date: '2026-04-24'
    status: active
    who: karolkow
    note: >
      Scope expanded from ADRs 0022-0031 to ALL current ADRs (0001-0036) per
      stakeholder request. Rationale: doing a one-shot catch-up sweep, handling
      only 10 ADRs leaves docs stale against the other 16 on merge day;
      no merit in two partial sweeps. Expanded matrix covers live process/infra
      ADRs (0001, 0002, 0004, 0005, 0006, 0007, 0008, 0010), the schema
      evolution chain 0011-0021 (mostly obsoleted by 0029), post-0031
      refinements (0033, 0034, 0035, 0036), and the evergreen policy (0032).
      ADR 0035 (drop `account_balance_history`) pre-applied to the docs
      ahead of its implementing task 0159 to prevent a docs↔migration race
      after 0159 lands. ADR 0033/0034 collateral promoted from "outside scope"
      to formally in-scope.
  - date: '2026-04-24'
    status: active
    who: karolkow
    note: >
      3rd pass — merged `origin/develop` into the 0155 branch to bring in
      task 0159's landing (dropped `account_balance_history` from
      migrations + indexer code + tests). Clean merge, zero conflicts.
      Pre-apply bet paid off: docs already matched the post-0159 reality,
      so the 3rd pass was pure cleanup — removed "migrations still carry"
      caveat in DB §4.18, tightened IX §5.2 step 14 narrative, flipped
      ADR 0035 row in matrix from `proposed` to `accepted`, added
      "Post-0155 backlog watch" section to matrix for future doc sweep
      triggers (tasks 0160, 0161, 0162, 0163 — none in 0155 scope, logged
      for reviewer awareness). No new ADRs 0037+ introduced on develop,
      so matrix and per-file worklog remain complete.
  - date: '2026-04-24'
    status: completed
    who: karolkow
    note: >
      Task complete. 3 passes + review pass + out-of-scope bonus fixes.
      Deliverables: 7 `docs/architecture/**` files reconciled (5 rewritten,
      1 minor sync, 1 no-change) + `docs/database-audit-first-implementation.md`
      stale-notice + `docs/audits/2026-04-10-pipeline-data-audit.md`
      stale-notice + `crates/db/MIGRATIONS.md` drift fix + ADR template
      delivery checklist + task template docs-updated criterion + root
      CLAUDE.md evergreen rule. Matrix covers ADRs 0001-0036 (26 live
      ADRs + 10 obsolete/process skipped with rationale). Worklog has
      per-file entries with verdict + diff summary + 2nd-pass + 3rd-pass
      sections. AC 8/10 satisfied (1 human PR review pending, 1 markdown
      lint N/A — project uses prettier which passes). Zero tests broken
      (doc-only change). No new ADRs needed.
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

**Full scope — every LIVE ADR in `lore/2-adrs/` as of 2026-04-24.** The
initial task body scoped only 0022-0031; the scope was widened on
2026-04-24 (see task history) because a partial sweep leaves the other
half of the ADRs unreflected on PR merge day, contradicting the spirit
of the ADR 0032 catch-up.

Process / infrastructure / API contract ADRs (LIVE, mostly IN/BE/IX
impact):

- 0001 — OIDC CI/CD + secret separation (IN)
- 0002 — Rust Ledger Processor Lambda (BE, IX, XD)
- 0004 — Rust-only XDR parsing (BE, XD)
- 0005 — Rust-only backend (BE)
- 0006 — no S3 lifecycle on ledger-data bucket (IN)
- 0007 — simplified 2-Lambda architecture (IN, BE)
- 0008 — error envelope + pagination shape (BE)
- 0010 — local backfill over Fargate (IX, IN)

Early schema evolution chain (mostly OBSOLETE — superseded by 0027 /
0029; surviving guidance documented where still live):

- 0011–0018 — S3-offload lightweight schema iterations (OBSOLETE via 0029)
- 0019 — schema snapshot + sizing reference (LIVE as capacity-planning baseline)
- 0020 — `transaction_participants` column cut, contract index cut (LIVE)
- 0021 — schema ↔ endpoint ↔ frontend coverage matrix (LIVE reference)

Core schema rework ADRs (deepest doc impact, primary sink of task 0155):

- 0022 — schema correction + token metadata enrichment
- 0023 — tokens typed metadata columns
- 0024 — hashes as BYTEA
- 0025 — final schema v1 (superseded by 0027)
- 0026 — accounts surrogate (BIGINT id)
- 0027 — post-surrogate schema + endpoint realizability (superseded by 0030)
- 0028 — parsed ledger artifact v1 (superseded by 0029 before ship)
- 0029 — abandon parsed artifacts in favour of read-time XDR fetch
- 0030 — contracts surrogate (BIGINT id)
- 0031 — enum columns SMALLINT + Rust enum

Governance + post-0031 refinements (all LIVE):

- 0032 — `docs/architecture/**` evergreen maintenance policy
- 0033 — `soroban_events_appearances` read-time detail
- 0034 — `soroban_invocations_appearances` read-time detail
- 0035 — drop `account_balance_history`
  (proposed; **pre-applied to docs** in this task to prevent a
  docs↔migration race when the implementing task 0159 merges — docs
  describe the post-drop shape; the migrations still carry the table
  until 0159 runs its `DROP TABLE`)
- 0036 — rename `tokens → assets` (already reflected in migrations via
  task 0154 pre-0155 baseline)

## Implementation Plan

### Step 1: Build the ADR → doc impact matrix

Full 26-ADR matrix produced at
[`notes/G-adr-doc-matrix.md`](notes/G-adr-doc-matrix.md). The matrix is the
checklist; it drives the sweep and prevents missed updates.

The matrix covers:

- **Process / infrastructure / API-contract ADRs** (0001, 0002, 0004, 0005,
  0006, 0007, 0008, 0010) — verify BE / IX / IN / XD overviews describe
  the accepted state
- **Schema evolution chain 0011–0021** — mostly superseded by 0029; keep
  surviving decisions (0019 sizing baseline, 0020 TP cut, 0021 coverage
  matrix) reflected in DB / TD
- **Core schema rework 0022–0031** — primary sink; drives rewrites in
  DB, TD §5/§6, XD, IX, IN
- **Evergreen policy 0032** — drives this task's Step 4 template work
- **Post-0031 refinements 0033–0036** — drive DB / TD / BE updates; 0035
  pre-applied

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

- [x] Worklog contains the ADR → doc impact matrix (Step 1 output).
      → `notes/G-adr-doc-matrix.md` (expanded 2026-04-24 to cover 0001-0036).
- [x] Every file under `docs/architecture/**` has a completed
      reconciliation pass, documented in the worklog (one entry per file
      with "no changes", "minor sync", or "rewritten — diff summary").
      → `notes/worklog.md` rows 1–8 + 2nd-pass summary.
- [x] Every LIVE ADR in `lore/2-adrs/` (0001-0036) has been walked;
      process-level ADRs with no doc surface (0003, 0009) explicitly
      marked as such in the matrix's "Out of scope" section.
- [x] Concrete drift points from the research note §5.2 are all
      addressed in `technical-design-general-overview.md` §6.7 (now
      §6.7 Assets) and in `database-schema-overview.md` §4.10 (post-rename).
- [x] Each rewritten section that reflects an ADR-driven change links
      to the relevant ADR(s) for context.
- [x] ADR 0035 pre-applied to the docs (account_balance_history dropped
      across 6 files) to prevent a docs↔migration race when task 0159 merges.
- [x] `lore/2-adrs/_template.md` and `lore/1-tasks/_template.md` updated
      with a "Docs updated" checklist entry.
- [x] Root `CLAUDE.md` documents the per-ADR maintenance rule defined
      by ADR 0032.
- [ ] PR review pass — a second team member confirms at least one
      per-file reconciliation against the current code, to catch
      blind-spot errors. _(pending reviewer)_
- [ ] Markdown lint (if the project has one) passes on all touched
      files. _(not yet run — project has no dedicated markdown-lint CI
      as of 2026-04-23)_

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

## Implementation Notes

**3 passes + review + out-of-scope bonus.** Delivered in a single day
(2026-04-24).

### Artifacts produced

- `notes/G-adr-doc-matrix.md` — 27-row matrix (ADRs 0001-0036) grouped by
  category: process/infra/API (0001-0010), schema evolution chain
  (0011-0021), core rework (0022-0031), governance + post-0031 (0032-0036).
  Includes backlog-watch section for tasks 0160-0163 flagged on develop.
- `notes/worklog.md` — per-file reconciliation log (rows 1-8) with
  verdicts (rewritten / minor sync / no change / stale-notice) plus
  2nd-pass and 3rd-pass summary sections.

### Files touched in `docs/architecture/**`

| File                                              | Verdict    | Scope                                                                                |
| ------------------------------------------------- | ---------- | ------------------------------------------------------------------------------------ |
| `database-schema/database-schema-overview.md`     | rewritten  | §1-§8 all touched; 18 table subsections (6 new); ADR 0035 drop-notice in §4.18       |
| `technical-design-general-overview.md`            | rewritten  | §2.2, §3, §4 pipeline + §4.3, §5 XDR parsing, §6 all 12 table blocks, ASCII diagrams |
| `xdr-parsing/xdr-parsing-overview.md`             | rewritten  | Every section — two-path parser model, appearance-index extraction, ADR 0029 framing |
| `indexing-pipeline/indexing-pipeline-overview.md` | rewritten  | §5.2 14-step `persist_ledger`, §5.3 inventory, §6 local backfill (ADR 0010)          |
| `infrastructure/infrastructure-overview.md`       | minor sync | §2.2, §2.3, §5.3, §5.4, §6.4, §8 — Rust parser + public archive read path + ADR 0010 |
| `backend/backend-overview.md`                     | minor sync | §4.1 surrogate-key + archive read bullets, §4.2, §7.1                                |
| `frontend/frontend-overview.md`                   | no changes | No schema-shape references; API field names preserved by ADR 0026/0030 surface lock  |

### Files touched outside `docs/architecture/**`

- `docs/database-audit-first-implementation.md` — historical-snapshot header + per-section pointers (not regenerated, kept as point-in-time artifact)
- `docs/audits/2026-04-10-pipeline-data-audit.md` — same treatment
- `crates/db/MIGRATIONS.md` — drift cleanup (0004 / 0005 / 0007 table refs, partitioned list)
- `lore/2-adrs/_template.md` — new Delivery Checklist section (7 doc boxes + link requirement)
- `lore/1-tasks/_template.md` — new "Docs updated" acceptance criterion
- `CLAUDE.md` (root) — new "Evergreen Architecture Docs" section linking ADR 0032

### Pre-apply decision

ADR 0035 (drop `account_balance_history`) was `proposed` when the 2nd pass ran.
Docs were written assuming the post-drop shape **ahead of** the implementing
task 0159's code landing. This was a deliberate bet to avoid a docs↔migration
race on PR merge day. The bet paid off: 0159 landed on develop the same day
as 0155's 2nd pass; the 3rd pass merged develop in, migrations caught up,
and the pre-apply language was cleaned up without incident.

## Design Decisions

### From Plan

1. **Matrix-first, sweep-second** (Step 1 → Step 2 per task body). Matrix
   as the contract keeps the sweep mechanical and prevents scope creep.
2. **Templates + root CLAUDE.md closing the loop** (Step 4 per task body).
   The ADR 0032 evergreen policy is unenforceable without the template
   hooks; adding them closes the door on future drift.
3. **Audit doc (`database-audit-first-implementation.md`) stale-notice
   instead of regeneration** — tracked as `minor sync` verdict with
   follow-up chip. Audit is a different kind of doc (snapshot with
   `file:line` refs) and mixing its regeneration into 0155 would bloat
   the PR.

### Emerged

4. **Scope expansion to ADRs 0001-0036** — task body scoped only 0022-0031;
   stakeholder widened on 2026-04-24. Rationale: partial sweep leaves the
   other half unreflected on merge day, contradicting the spirit of the
   ADR 0032 catch-up. Matrix was extended, worklog §"2nd pass" added,
   4 new docs touched (IN, IX, backend) for ADR 0010 drift.
5. **ADR 0035 pre-apply** — normally docs follow code, but task 0159 was
   scheduled to land concurrently with 0155's PR. Pre-applying the drop
   to docs eliminated the race; the 3rd-pass merge confirmed it worked.
6. **ADR 0033/0034 collateral → formally in-scope** — the old tables
   (`soroban_events`, `soroban_invocations`) don't exist in migrations,
   so the docs had to describe the `_appearances` counterparts to be
   internally consistent. 2nd pass scope expansion promoted this from
   "collateral" to "in-scope".
7. **`crates/db/MIGRATIONS.md` + `docs/audits/2026-04-10-pipeline-data-audit.md`
   fixes outside `docs/architecture/**`\*\* — stakeholder bonus request.
   Found same drift patterns outside the formal scope; fixed in-pass to
   avoid leaving known drift behind.
8. **Audit doc chip follow-up dismissed** — stakeholder chose to treat the
   audit as a permanent historical snapshot, not a document to regenerate.
   Worklog entry for file #2 and audit-doc header both explicitly state
   "no regeneration planned".
9. **`docs/audits/2026-04-10-pipeline-data-audit.md` same treatment** —
   same kind of doc (point-in-time audit), same reasoning applied; just
   a header stale-notice.
10. **Research-notes stash** — user added 8 `R-*.md` research notes
    (totalling ~4k lines) between sessions; agreed these are out of
    scope for the 0155 PR and stashed them (`git stash@{0}`) rather
    than committing. Recoverable via `git stash pop`.

## Issues Encountered

- **Git stash -u + pathspec quirk**: `git stash push -u -- <paths>` captures
  all untracked in working tree, ignoring pathspec. Workaround: `git add`
  the files first, then `git stash push -- <paths>` (no `-u`). Fresh stash
  contained only the target files, not collateral.
- **`git stash show --stat` showed more than expected**: default compares
  stash state to HEAD, which includes every difference, even unaffected.
  Misleading but not a bug. Working tree verification confirmed the stash
  had removed only the target files.
- **1st-pass spurious column in DB §4.5**: introduced `role SMALLINT` on
  `transaction_participants`. Caught in 2nd pass by re-verifying against
  migration 0003. Real table is 3-col per ADR 0020.
- **1st-pass Fargate references for backfill (TD, IX, IN)**: narrative
  inherited from pre-ADR-0010 docs. Caught in 2nd pass; ADR 0010 is LIVE,
  backfill is local CLI. Fixed.
- **Review-pass finding #1 (constraint name drift)**: DB §4.13 had
  `ck_nft_ownership_event_type_range`; migration uses `ck_nft_own_event_type_range`.
  Fixed in review pass.
- **Review-pass findings #2/#3 (stale "outside scope" language, ADR ref
  precision)**: DB §4.8/§4.9 and TD §6.5 still contained 1st-pass
  "outside this audit's 0022-0031 scope" language that was stale after
  the 2nd-pass scope expansion. Fixed in review pass.

## Future Work

None spawned by 0155. Tasks already on develop (0160/0161/0162/0163) are
the natural follow-ups to the docs-reflecting-code sync, but they are
owned by their respective authors and will update docs per the ADR 0032
checklist at their own merge time. Logged in the matrix "Post-0155
backlog watch" section for reviewer awareness.
