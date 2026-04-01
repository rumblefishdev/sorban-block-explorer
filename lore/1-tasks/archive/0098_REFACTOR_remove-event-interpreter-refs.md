---
id: '0098'
title: 'Cleanup: remove Event Interpreter references from backlog and docs'
type: REFACTOR
status: completed
related_adr: ['0005', '0007']
related_tasks: ['0028', '0046', '0050', '0070', '0088']
tags: [priority-medium, effort-small, layer-docs]
milestone: 1
links: []
history:
  - date: 2026-04-01
    status: backlog
    who: stkrolikiewicz
    note: 'Task created — Event Interpreter removed from architecture (2 Lambdas only). Cleanup remaining references in backlog tasks and docs.'
  - date: 2026-04-01
    status: active
    who: stkrolikiewicz
    note: 'Activated — cleanup Event Interpreter refs from backlog and docs'
  - date: 2026-04-01
    status: done
    who: stkrolikiewicz
    note: >
      Completed all 4 steps. Deleted 0056 + apps/workers/ (moved to .trash/).
      Updated 5 backlog tasks (0028, 0046, 0050, 0070, 0088), 4 architecture docs,
      wiki snapshot. Regenerated lore index. Added ADR 0007 (simplified 2-Lambda architecture).
      Removed apps/workers tsconfig reference. Grep verification: zero functional hits.
---

# Cleanup: remove Event Interpreter references from backlog and docs

## Summary

Event Interpreter Lambda was removed from the architecture (commit 7c961e6). Active tasks (0018, 0026, 0033) are being updated separately. This task cleans up remaining references in backlog tasks, architecture docs, and the lore index.

## Context

The simplified architecture has 2 Lambdas (API + Indexer), no Event Interpreter. The `event_interpretations` table has no producer and should be removed from schema docs and task descriptions. Several backlog tasks still reference LEFT JOINs on this table, `human_readable` fields, and task 0056.

## Implementation Plan

### Step 1: Delete obsolete files

- `lore/1-tasks/backlog/0056_FEATURE_workers-event-interpreter.md` → cancel or move to `.trash/`
- `apps/workers/` → move to `.trash/` (TS stub, sole purpose was event interpretation)

### Step 2: Update backlog tasks

- **0028** — remove `soroban_events -> event_interpretations` from CASCADE chain
- **0046** — remove LEFT JOIN `event_interpretations`, all `human_readable` fields from response examples, AC about event_interpretations, notes about enrichment deferral
- **0050** — remove `interpretation` field from response, `human_readable` from response example, LEFT JOIN references, AC, notes about enrichment deferral
- **0070** — remove `human_readable` from response example JSON, remove note about summaries coming from `human_readable` field
- **0088** — remove disclaimers about "Event Interpreter tests deferred" and "no separate Event Interpreter Lambda in current architecture"

### Step 3: Update architecture docs

- **`docs/architecture/database-schema/database-schema-overview.md`** — remove section 4.7 (Event Interpretations DDL), `event_interpretations` from relationship diagram, mentions of enrichment layer and periodic enrichment writes
- **`docs/architecture/technical-design-general-overview.md`** — remove section 6.7 (Event Interpretations), `human_readable` from response example, mentions of event interpretation in testing/risks sections
- **`docs/architecture/backend/backend-overview.md`** — remove `human_readable` from response example, mentions of "event interpretations" in response shaping, "readable interpretations" and "structured interpretations" references, "interpretation responsibility" mention
- **`docs/architecture/xdr-parsing/xdr-parsing-overview.md`** — remove mention of event interpretation jobs working from persisted events

### Step 4: Regenerate lore index

- Run `lore-framework_generate-index` to update `lore/README.md` (currently lists 0056 as backlog task)

## Out of scope

- **Active tasks (0018, 0026, 0033)** — updated separately by their assignees
- **Archived tasks (0002, 0005, 0006, 0008, 0010, 0085, 0093)** — closed, history preserved as-is
- **Already updated tasks (0036, 0037, 0040)** — only have history entries noting the removal, no stale content
- **Tasks 0044, 0065** — mention "human-readable" in UI/response shaping context, not Event Interpreter
- **`.claude/skills/`** — pr and branch skills reference archived 0008 as examples, not functional dependencies

## Acceptance Criteria

- [x] Task 0056 canceled or deleted
- [x] `apps/workers/` removed
- [x] Backlog tasks 0028, 0046, 0050, 0070, 0088 updated — no references to `event_interpretations`, `Event Interpreter`, or task 0056
- [x] Architecture docs updated — no `event_interpretations` DDL, no Event Interpreter mentions
- [x] `lore/README.md` regenerated without 0056
- [x] Wiki snapshot updated to reflect simplified architecture (no Event Interpreter, no `event_interpretations` table)
- [x] Verification: grep for `event_interpretations`, `Event Interpreter`, `human_readable`, `0056` across non-archive, non-active files returns zero functional hits (history entries and task 0098 itself excluded)

## Implementation Notes

- Deleted files moved to `.trash/` per project policy: `0056_FEATURE_workers-event-interpreter.md`, `apps/workers/`
- Removed `apps/workers` project reference from root `tsconfig.json`
- Created ADR 0007 documenting the simplified 2-Lambda architecture
- All 4 architecture docs cleaned, all 5 backlog tasks cleaned
- Remaining grep hits are only `history:` entries in backlog tasks (documenting the removal itself) — allowed by AC

## Design Decisions

### From Plan

1. **Move to .trash/ instead of rm**: Per project file deletion policy.
2. **History entries preserved**: Backlog task history entries mentioning Event Interpreter kept as audit trail.

### Emerged

3. **tsconfig.json cleanup**: Plan didn't mention tsconfig, but `apps/workers` project reference would cause build errors. Added as final commit.
4. **ADR 0007 created**: Formalized the 2-Lambda architecture decision that was implicit in the removal.
