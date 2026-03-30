---
id: '0003'
title: 'Milestone-based task ordering: add milestone field and reindex backlog by deliverable'
status: accepted
deciders: [fmazur]
related_tasks: ['0085']
related_adrs: []
tags: [process, planning]
links:
  - docs/architecture/technical-design-general-overview.md
history:
  - date: 2026-03-30
    status: accepted
    who: fmazur
    note: 'ADR created. Initial decision: milestone field only, no renumbering.'
  - date: 2026-03-30
    status: accepted
    who: fmazur
    note: 'Updated: full reindex performed in addition to milestone field. All backlog IDs renumbered to follow M1→M2→M3 order. All cross-references updated.'
---

# ADR 0003: Milestone-based task ordering — add milestone field and reindex backlog by deliverable

**Related:**

- [Task 0085: Reindex backlog tasks by deliverable milestone](../1-tasks/archive/0085_REFACTOR_backlog-milestone-ordering.md)

---

## Context

The technical design (§7.4) defines three deliverables that must be completed in order: D1 (Indexing Pipeline & Core Infra, 20%), D2 (Complete API + Frontend, 30%), D3 (Mainnet Launch, 40%).

Backlog tasks were created numbered by layer — DB (0015–0022), API (0023–0038), Frontend (0039–0059), Indexer (0060–0066), Infra (0068–0078). This meant D1 tasks (indexer, CDK infra) had the highest IDs even though D1 must be completed first. Reading the backlog in ID order gave no indication of delivery priority.

---

## Decision

Two changes, applied together:

1. **Add `milestone: N` field** (N = 1, 2, or 3) to every task's YAML frontmatter — backlog, active, and archive. The field maps directly to the Three-Milestone Delivery Plan.

2. **Reindex all backlog task IDs** so that numbering follows milestone order:

   - M1 tasks: 0016–0042 (27 tasks)
   - M2 tasks: 0043–0077, 0086–0087 (37 tasks, gap at 0079–0085 for archive/active)
   - M3 tasks: 0088–0090 (3 tasks)

   All `related_tasks` cross-references across backlog, archive, and active were updated to new IDs. Dependency order within each milestone was verified (no task references a same-milestone task with a higher ID, except mutual relationships).

---

## Rationale

The initial decision was milestone field only (no renumbering), to avoid breaking cross-references and git history. However, after analysis:

- The project is early — only 2 tasks have commits (0015 active, 0085 meta-task). Git history breakage is minimal.
- Having IDs match delivery order makes the backlog immediately scannable. "Task 0030" is obviously D1 work, "task 0070" is obviously D2 frontend.
- The milestone field alone didn't solve the UX problem — you still had to mentally ignore ID order when planning work.
- A scripted bulk rename with automated `related_tasks` remapping made the migration safe (verified: 0 dangling refs, 0 mismatches).

Both mechanisms reinforce each other: IDs give quick visual ordering, milestone field gives machine-readable metadata.

---

## Alternatives Considered

### Alternative 1: Milestone field only (no renumbering)

**Description:** Add `milestone: N` to frontmatter but keep original task IDs.

**Pros:**

- Zero risk of broken references
- No changes to archive tasks
- Git commit scopes (`lore-NNNN`) remain valid

**Cons:**

- IDs still don't reflect delivery order — confusing when reading the backlog
- Milestone field alone doesn't help visual scanning
- The `lore-framework-mcp` generator doesn't sort by milestone, so `README.md` board still shows ID order

**Decision:** PARTIALLY ADOPTED — milestone field added, but renumbering also performed because the project is early enough to absorb the cost.

### Alternative 2: Priority tags (e.g., `milestone-1`, `milestone-2`)

**Description:** Encode deliverable in existing `tags` array.

**Pros:**

- No schema change

**Cons:**

- Tags are unstructured strings — no type safety
- Mixes concerns: priority tags vs delivery ordering

**Decision:** REJECTED — milestone is a distinct concept deserving its own field.

---

## Consequences

### Positive

- Task IDs directly reflect delivery order — the backlog is self-documenting
- Milestone field provides machine-readable metadata for filtering and sorting
- All cross-references verified correct after migration (0 dangling, 0 mismatches)
- Dependency order within milestones is respected (verified: idempotent writes before handler, MUI theme before UI components)
- New tasks created for previously missing scope (Galexie config, Swagger infra, D3 tests)

### Negative

- Git commits referencing old task IDs (before this change) use stale scopes — only affects tasks 0015 and 0085 which kept their IDs
- Archive tasks were edited to update `related_tasks` — acceptable since these are metadata changes, not content changes
- The `lore-framework-mcp` index generator does not yet sort by `milestone` — manual/Claude sorting remains necessary
- Gap at 0079–0085 in M2 range (archive/active IDs) breaks otherwise continuous sequence
