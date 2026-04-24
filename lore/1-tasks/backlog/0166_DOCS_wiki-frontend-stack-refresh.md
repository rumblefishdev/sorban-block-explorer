---
id: '0166'
title: 'DOCS: Fix misassigned task IDs + refresh wiki frontend-stack snapshot'
type: DOCS
status: backlog
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
---

# DOCS: Fix misassigned task IDs + refresh wiki frontend-stack snapshot

## Summary

`lore/3-wiki/project/frontend-stack.md` contains four task-ID references
that all point at tasks whose actual content is unrelated to the feature
claimed next to them (legacy numbering from before backlog restructuring).
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
| "UI components (tasks 0040–0045)"   | Range spans unrelated backend tasks          | Umbrella: 0058–0076 + 0086/0087 |

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

- [ ] Every `task NNNN` reference in `frontend-stack.md` points at a task
      whose content actually matches the described feature
- [ ] Package versions in the stack table verified against `web/package.json`
- [ ] "What is ready" / "What does not exist yet" sections reflect current state
- [ ] Prettier-clean
- [ ] **Docs updated** — N/A — this task IS the doc update. No
      `docs/architecture/**` change needed.

## Notes

Scope intentionally narrow. A full rewrite analogous to 0165 would be
premature: the frontend has not undergone ADR-driven architectural shifts
that drove the backend snapshot drift. Revisit if/when the UI backlog
(0058–0087) starts landing in volume.
