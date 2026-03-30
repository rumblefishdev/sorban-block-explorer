# Web

React SPA for the Soroban Block Explorer frontend.

## Stack

- **React 19** with `react-dom/client` (`createRoot`)
- **Vite** as dev server and production bundler
- **MUI** as base component library and accessibility layer
- **React Router** for client-side routing
- **TanStack Query** for server-state fetching and caching

## Development

```bash
npx nx dev @rumblefish/soroban-block-explorer-web   # dev server on localhost:4200
npx nx build @rumblefish/soroban-block-explorer-web  # production build to dist/
npx nx lint @rumblefish/soroban-block-explorer-web
npx nx typecheck @rumblefish/soroban-block-explorer-web
```

## Structure

```text
apps/web/
  index.html          # SPA entry point
  vite.config.ts      # Vite app config with React plugin
  src/
    main.tsx          # React root render (StrictMode)
    app.tsx           # Root App component
```

## Workspace Imports

Uses `@rumblefish/soroban-block-explorer-ui` for shared UI components and
`@rumblefish/soroban-block-explorer-domain` for domain types. Cross-project
imports resolve via the `soroban-block-explorer-source` export condition.
