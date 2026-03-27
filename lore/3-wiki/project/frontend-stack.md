# Frontend Stack

> Current state of the frontend environment in the workspace.

## apps/web

React 19 SPA served by Vite. Entry point: `index.html` → `src/main.tsx` → `src/app.tsx`.

| Technology       | Packages                                                                    | Role                      |
| ---------------- | --------------------------------------------------------------------------- | ------------------------- |
| React 19         | `react`, `react-dom`                                                        | UI rendering              |
| Vite 7           | `vite`, `@vitejs/plugin-react`                                              | Dev server + bundler      |
| MUI 7            | `@mui/material`, `@mui/icons-material`, `@emotion/react`, `@emotion/styled` | Components, theming, a11y |
| React Router 7   | `react-router-dom`                                                          | Client-side routing       |
| TanStack Query 5 | `@tanstack/react-query`, `@tanstack/react-query-devtools`                   | Fetching, cache, polling  |

### Configuration

- `vite.config.ts` — app mode, port 4200, `resolve.conditions: ['soroban-block-explorer-source']`
- `tsconfig.lib.json` — `jsx: "react-jsx"`, `lib: ["es2022", "dom", "dom.iterable"]`
- `eslint.config.mjs` — covers `*.ts`, `*.tsx`, `*.js`, `*.jsx`

### What is ready

- Bootstrap: React root with `StrictMode`, minimal `App` component
- Import from `@rumblefish/soroban-block-explorer-ui` works
- `nx build`, `nx lint`, `nx typecheck`, `nx dev` — all passing

### What does not exist yet

- Routing (task 0047)
- TanStack Query provider and API client (task 0046)
- MUI theme (task 0077)
- Layout shell, header, navigation (task 0039)

## libs/ui

Shared React component library in Vite lib mode.

- `vite.config.ts` — `build.lib`, ES format, externalizes react/mui/emotion
- Currently exports only the `NavigationItem` interface
- UI components will be added in follow-up tasks (0039, 0040–0045)

## Key workspace patterns

- **No project.json** — targets inferred by Nx plugins from `vite.config.ts`, `tsconfig.lib.json`, `eslint.config.mjs`
- **No tsconfig path aliases** — imports use npm package names (`@rumblefish/soroban-block-explorer-*`) with custom export condition `soroban-block-explorer-source`
- **Internal imports use `.js` extension** — required by `moduleResolution: "nodenext"` (TS resolves `.js` → `.tsx`)
- **Dependencies in root `package.json`** — npm workspaces with hoisting, no per-project dependencies
