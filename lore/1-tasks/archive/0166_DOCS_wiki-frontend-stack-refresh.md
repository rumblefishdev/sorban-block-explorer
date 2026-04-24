---
id: '0166'
title: 'DOCS: Fix misassigned task IDs + refresh wiki frontend-stack snapshot'
type: DOCS
status: completed
related_adr: []
related_tasks: ['0165', '0058', '0059', '0066', '0067', '0077', '0084']
tags: ['phase-maintenance', 'effort-small', 'priority-low', 'wiki', 'frontend']
links:
  - lore/3-wiki/project/frontend-stack.md
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0165 future work. Every `task NNNN` reference in the
      current frontend-stack.md points at a task whose content is unrelated
      to the feature described — legacy numbering from before the backlog
      was restructured. Fix the mapping and reconcile descriptions with
      actual frontend state.
  - date: '2026-04-24'
    status: active
    who: stkrolikiewicz
    note: 'Promoted to active.'
  - date: '2026-04-24'
    status: completed
    who: stkrolikiewicz
    note: >
      5 task-ID remappings applied + 1 new entry (0096 OpenAPI TS codegen).
      All config claims verified against repo HEAD. Prettier-clean.
---

# DOCS: Fix misassigned task IDs + refresh wiki frontend-stack snapshot

## Summary

`lore/3-wiki/project/frontend-stack.md` contains four misassigned single
task-ID references plus one misassigned task range, all pointing at tasks
whose actual content is unrelated to the feature claimed next to them
(legacy numbering from before backlog restructuring).
The factual stack description (React 19, Vite 7, MUI 7, TanStack Query 5,
React Router 7) and the "bootstrap-only" characterisation remain accurate
— the frontend is still at `web/src/{app,main}.tsx` + `libs/ui/src/index.ts`.

## Context

Misassigned references found during task 0165 gap audit:

| Claim in frontend-stack.md          | Referenced task (real content)               | Correct current task            |
| ----------------------------------- | -------------------------------------------- | ------------------------------- |
| "Routing (task 0047)"               | 0047 = Backend Ledgers Module (backlog)      | **0067** router + routes        |
| "TanStack Query client (task 0046)" | 0046 = Backend Transactions Module (archive) | **0066** TanStack Query client  |
| "MUI theme (task 0077)"             | 0077 = Frontend LP list/detail (backlog)     | **0058** UI MUI theme           |
| "Layout shell (task 0039)"          | 0039 = CI/CD GitHub Actions (archive)        | **0059** layout shell + nav     |
| "UI components (tasks 0040–0045)"   | Range spans unrelated backend tasks          | Umbrella: 0058–0077, 0086, 0087 |

Archived frontend-related tasks that DID land:

- **0042** — OpenAPI + Swagger infrastructure (backend-side, enables frontend codegen)
- **0084** — Frontend bootstrap Nx scaffold (the current `web/` skeleton)
- **0096** — OpenAPI → TypeScript codegen (backlog promotion status to verify at task time)

## Implementation

1. Open `lore/3-wiki/project/frontend-stack.md`.
2. Replace each misassigned task reference per the table above.
3. Verify versions in the stack table match current `web/package.json`
   (React 19, Vite 7, MUI 7, TanStack Query 5, React Router 7).
4. Confirm "What is ready" still matches actual `web/src/` state (likely
   unchanged: StrictMode root, minimal `App`, nothing beyond scaffold).
5. Confirm "What does not exist yet" still accurate — per backlog,
   virtually none of the UI work (0058–0087) has shipped since 0084.
6. Apply Prettier.

## Acceptance Criteria

- [x] Every `task NNNN` reference in `frontend-stack.md` points at a task
      whose content actually matches the described feature
- [x] Package versions in the stack table verified against `web/package.json`
- [x] "What is ready" / "What does not exist yet" sections reflect current state
- [x] Prettier-clean
- [x] **Docs updated** — N/A — this task IS the doc update. No
      `docs/architecture/**` change needed.

## Implementation Notes

Five edits to `lore/3-wiki/project/frontend-stack.md`:

1. Routing: `task 0047` → `task 0067` (frontend-router-routes)
2. TanStack Query: `task 0046` → `task 0066` (frontend-tanstack-query-api-client)
3. MUI theme: `task 0077` → `task 0058` (ui-mui-theme)
4. Layout shell: `task 0039` → `task 0059` (ui-layout-shell-header-nav)
5. UI components range: `(0039, 0040–0045)` → `(tasks 0058–0077, 0086, 0087)`

Plus one new entry added during gap audit:

6. **API type generation from OpenAPI (task 0096)** appended to "What does
   not exist yet" — frontend currently has no generated client types;
   affects how FE consumes backend types.

Verifications performed against repo HEAD:

- `web/package.json` — only `tslib`; all FE deps hoisted to root per npm
  workspaces pattern. Root `package.json` confirms React 19.2.4, Vite 7,
  MUI 7.3.9, TanStack Query 5.95.2, React Router 7.13.2.
- `web/vite.config.ts` — port 4200, `resolve.conditions: ['soroban-block-explorer-source']`.
- `web/tsconfig.lib.json` — `jsx: "react-jsx"`, `lib: ["es2022", "dom", "dom.iterable"]`.
- `web/eslint.config.mjs` — covers `.ts`, `.tsx`, `.js`, `.jsx`.
- `web/src/` — only `app.tsx` + `main.tsx`; `libs/ui/src/index.ts` exports
  only `NavigationItem`.
- `libs/ui/vite.config.ts` — `build.lib`, `formats: ['es']`, externalizes
  react/mui/emotion.
- `tsconfig.base.json` — `moduleResolution: "nodenext"` (backs up `.js`
  extension claim).
- No `project.json` under `web/` or `libs/` (Nx plugin inference claim
  confirmed).

## Design Decisions

### From Plan

1. **Narrow scope — fix references, don't rewrite.** Unlike 0165 (where the
   system had fundamentally shifted), the frontend has not undergone
   ADR-driven drift; the factual description still holds. Scope limited to
   remapping misassigned IDs and verifying configuration claims.

### Emerged

2. **Added task 0096 (OpenAPI → TS codegen) to "What does not exist yet".**
   Not in the original plan, but discovered during gap audit: the frontend
   has no generated API types and 0096 is the task that introduces them.
   A reader looking at "What does not exist yet" would otherwise miss a
   material FE gap.
3. **Included task 0077 in the UI components range (0058–0077 continuous).**
   Initial remap listed `0058–0076, 0086, 0087`; user challenge during gap
   audit surfaced that 0077 is the frontend LP list/detail task (same
   bucket), so the correct continuous range is `0058–0077, 0086, 0087`.
4. **Kept `## Status: Backlog` out of task frontmatter.** Copilot PR
   review for 0165 suggested this stylistic section; checked 58 backlog
   files — convention is split, Summary already captures current state.
   Decision carried into 0166.

## Future Work

- None immediate. Revisit frontend-stack snapshot once the UI backlog
  (0058–0087) starts landing in volume, or if a frontend ADR emerges.
