---
id: '0039'
title: 'UI lib: layout shell, header, navigation, network indicator'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0077']
tags: [priority-high, effort-medium, layer-frontend-shared]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# UI lib: layout shell, header, navigation, network indicator

## Summary

Implement the persistent layout shell for the explorer frontend in `libs/ui/src/layout/`. This includes the header (logo, global search bar slot, network indicator), primary navigation, environment banner, and the content area wrapper. The shell remains stable across route transitions -- only the content area updates on navigation.

## Status: Backlog

**Current state:** Not started.

## Context

The explorer frontend needs a consistent, always-visible layout shell that frames every page. The shell provides orientation (where am I, what network), quick access to all entity categories, and the global search entrypoint. Without a stable shell, route transitions cause white-screen reloads and users lose navigation context.

Design philosophy from the architecture docs:

- Data-first, explorer-oriented layout. Scanability over marketing.
- Collection screens are list-heavy; detail screens show concise summary first.
- Route transitions must preserve shell rendering -- no white-screen reload. Route changes update the content area while header/nav remain stable.
- Network indicator always visible to prevent mainnet/testnet confusion.

## Implementation Plan

### Step 1: Layout shell wrapper

Create `libs/ui/src/layout/AppShell.tsx` providing the outer frame: header region, navigation region, and `<main>` content outlet. Use semantic HTML (`<header>`, `<nav>`, `<main>`). The content area accepts `children` (React Router `<Outlet>`).

### Step 2: Header component

Create `libs/ui/src/layout/Header.tsx`:

- Logo (left-aligned, links to `/`)
- Global search bar slot -- renders the search bar component from task 0040 via a prop or composition slot
- Network indicator (right-aligned) showing "Mainnet" or "Testnet" with distinct visual treatment (color, badge). Must be visible at all times.
- Environment banner for non-production environments (e.g., "TESTNET" banner)

### Step 3: Navigation component

Create `libs/ui/src/layout/Navigation.tsx`:

- Links to: Home (`/`), Transactions (`/transactions`), Ledgers (`/ledgers`), Tokens (`/tokens`), Contracts (`/contracts`), NFTs (`/nfts`), Liquidity Pools (`/liquidity-pools`)
- Active link highlighting based on current route
- Responsive: collapsible on small screens

### Step 4: Network indicator component

Create `libs/ui/src/layout/NetworkIndicator.tsx`:

- Reads current network from app config/context
- Renders mainnet/testnet badge using MUI theme palette from task 0077
- Always visible in header

### Step 5: Integration and exports

Export all layout components from `libs/ui` barrel. Ensure the shell works with React Router `<Outlet>` for content area rendering.

## Acceptance Criteria

- [ ] AppShell renders header, navigation, and content area using semantic HTML (`<header>`, `<nav>`, `<main>`)
- [ ] Header displays logo, search bar slot, and network indicator
- [ ] Navigation contains links to all seven top-level routes with active state
- [ ] Network indicator shows mainnet or testnet and is always visible
- [ ] Environment banner renders for non-production environments
- [ ] Route transitions update only the content area; shell does not unmount/remount
- [ ] Navigation is keyboard-accessible with proper tab order
- [ ] Components use MUI theme from task 0077
- [ ] All components exported from `libs/ui`

## Notes

- The global search bar component itself is task 0040; this task provides the slot/composition point for it.
- MUI theme configuration is task 0077; this task consumes that theme.
- Navigation link list should be easy to extend if new entity types are added later.
