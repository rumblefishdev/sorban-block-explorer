---
id: '0084'
title: 'Frontend bootstrap: scaffold apps/web and libs/ui with core dependencies'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0039', '0046', '0047', '0077']
tags: [priority-high, effort-small, layer-frontend-shared]
links: []
history:
  - date: 2026-03-27
    status: backlog
    who: fmazur
    note: 'Task created — discovered missing bootstrap step while reviewing frontend backlog'
  - date: 2026-03-27
    status: active
    who: fmazur
    note: 'Activated — prerequisite for all frontend tasks'
  - date: 2026-03-27
    status: completed
    who: claude
    note: >
      Bootstrap complete. 5 new files, 7 modified files, 1 deleted.
      Installed 12 packages (react, react-dom, MUI, emotion, react-router-dom,
      tanstack-query). All 8 acceptance criteria met. Key decisions:
      manual transform instead of Nx generators, @vitejs/plugin-react@5
      due to Vite 7 compat.
---

# Frontend bootstrap: scaffold apps/web and libs/ui with core dependencies

## Summary

Scaffold the frontend application (`apps/web`) and shared UI library (`libs/ui`) within the Nx workspace, and install all core dependencies needed by downstream frontend tasks. This is the foundational setup that every other frontend task assumes already exists.

## Status: Completed

## Context

Existing frontend tasks (0039, 0046, 0047, 0077) all create files inside `apps/web/` and `libs/ui/`, but no task existed to scaffold these projects or install the required dependencies. Without this bootstrap step, none of the frontend tasks could begin.

## Acceptance Criteria

- [x] `apps/web/` exists as an Nx React app with Vite bundler
- [x] `libs/ui/` exists as an Nx React library
- [x] MUI packages installed: `@mui/material`, `@mui/icons-material`, `@emotion/react`, `@emotion/styled`
- [x] React Router installed: `react-router-dom`
- [x] TanStack Query installed: `@tanstack/react-query`, `@tanstack/react-query-devtools`
- [x] `nx build web` and `nx build ui` succeed
- [x] `apps/web` can import from `libs/ui`
- [x] `nx dev web` starts dev server and renders the app

## Implementation Notes

### New files (5)

- `apps/web/vite.config.ts` — app mode, React plugin, port 4200, `resolve.conditions`
- `apps/web/index.html` — SPA entry point with `<div id="root">`
- `apps/web/src/main.tsx` — React 19 `createRoot` + `StrictMode`
- `apps/web/src/app.tsx` — minimal App component importing `NavigationItem` from libs/ui
- `libs/ui/vite.config.ts` — lib mode, React plugin, externalizes react/mui/emotion

### Modified files (7)

- `package.json` (root) — added runtime deps (react, mui, router, tanstack) + dev deps (@types/react, plugin-react, @nx/react)
- `eslint.config.mjs` (root) — added `*.tsx`/`*.jsx` to both file pattern arrays
- `apps/web/package.json` — removed library fields (`main`, `module`, `types`, `exports`)
- `apps/web/tsconfig.lib.json` — added `jsx: "react-jsx"`, `lib: ["es2022", "dom", "dom.iterable"]`, `src/**/*.tsx` include
- `apps/web/eslint.config.mjs` — added `*.tsx`/`*.jsx` patterns
- `libs/ui/tsconfig.lib.json` — same JSX/DOM/tsx changes as apps/web
- `libs/ui/eslint.config.mjs` — added `*.tsx`/`*.jsx` patterns

### Deleted files (1)

- `apps/web/src/index.ts` — old stub (moved to `.trash/`)

### Documentation updated (4)

- `apps/web/README.md` — replaced "Placeholder" with full stack/commands/structure description
- `README.md` (root) — updated Current Status section to reflect frontend bootstrap
- `docs/architecture/frontend/frontend-overview.md` — removed "still skeletal" language (2 places)
- `lore/3-wiki/project/frontend-stack.md` — new wiki entry documenting current frontend state

### Dependencies installed

**Runtime:** `react`, `react-dom`, `@mui/material`, `@mui/icons-material`, `@emotion/react`, `@emotion/styled`, `react-router-dom`, `@tanstack/react-query`, `@tanstack/react-query-devtools`

**Dev:** `@types/react`, `@types/react-dom`, `@vitejs/plugin-react@^5`, `@nx/react@22.6.1`

## Issues Encountered

- **`@vitejs/plugin-react@6` requires Vite 8**: Workspace uses Vite 7.3.1. npm install failed with peer dependency conflict. Fix: pinned to `@vitejs/plugin-react@^5` which supports Vite 7.

- **Nx project names are full package names**: `npx nx build ui` failed with "Cannot find project 'ui'". Projects are registered as `@rumblefish/soroban-block-explorer-ui`. All nx commands require the full package name.

- **TypeScript reference sync required after cross-project import**: Adding import from `@rumblefish/soroban-block-explorer-ui` in apps/web triggered `nx sync` requirement. Nx automatically added the tsconfig reference via `@nx/js:typescript-sync`.

## Design Decisions

### From Plan

1. **Manual transform instead of Nx generators**: `apps/web` and `libs/ui` already existed as stubs with package.json, tsconfig, eslint configs. Running `@nx/react:app` or `@nx/react:lib` generators would conflict with existing files. Manually added React-specific configs to existing skeletons instead.

2. **`resolve.conditions: ['soroban-block-explorer-source']` in Vite configs**: Mirrors `customConditions` in tsconfig.base.json. Enables Vite to resolve workspace imports (`@rumblefish/soroban-block-explorer-*`) to source files instead of dist/.

3. **No tsconfig path aliases**: Workspace convention uses npm package names with custom export conditions for cross-project imports. Kept this pattern for frontend projects.

4. **`module: "nodenext"` preserved**: Compatible with `jsx: "react-jsx"` in TypeScript 5.9. Internal imports use `.js` extension for `.tsx` files per workspace convention.

5. **libs/ui in Vite lib mode with externalized dependencies**: react, react-dom, MUI, and emotion externalized — consumers (apps/web) provide them. Prevents duplicate bundling.

### Emerged

6. **`@vitejs/plugin-react@^5` instead of latest @6**: Discovered at install time that v6 requires Vite 8. Downgraded to v5 which supports Vite 7. No functional difference for the bootstrap scope.

7. **Removed library fields from apps/web/package.json**: The stub had `main`, `module`, `types`, `exports` — these are library publication fields that have no meaning for an application. Removed to avoid confusion. libs/ui retains them (it IS a library).

8. **Added placeholder navigation in App component**: Included a minimal `NavigationItem[]` usage in `app.tsx` to verify cross-project imports work end-to-end, not just at typecheck level.

## Notes

- This task MUST be completed before tasks 0039, 0046, 0047, and 0077 can start.
- Do not add routing, theme, or providers in this task — keep it minimal. Those are covered by dedicated tasks.
