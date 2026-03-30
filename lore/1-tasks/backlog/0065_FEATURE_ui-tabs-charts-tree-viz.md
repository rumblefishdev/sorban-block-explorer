---
id: '0065'
title: 'UI lib: tabs, charts, and graph/tree visualization primitives'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-large, layer-frontend-shared]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# UI lib: tabs, charts, and graph/tree visualization primitives

## Summary

Implement tabs, time-series charts, operation flow tree/graph, and Soroban invocation call tree visualization components in `libs/ui/src/visualization/`. These are the most complex shared UI primitives in the explorer, used on transaction detail, contract detail, and liquidity pool detail pages.

## Status: Backlog

**Current state:** Not started.

## Context

Several explorer pages require rich visualization beyond simple tables:

- Transaction detail needs an operation flow tree (normal mode) and a Soroban invocation call tree
- Liquidity pool detail needs time-series charts for TVL, volume, and fee revenue
- Contract detail and transaction detail use tabbed sections

Design constraints:

- Tabs: no hard reloads, active tab reflected in URL
- Charts: time-series line/area for TVL, volume, fee revenue. Interval selector (1h/1d/1w) + date range. Responsive for small screens.
- Operation flow tree: source account to operations to affected accounts/contracts. Each node shows human-readable summary (e.g., "Sent 1,250 USDC to GD2M...K8J1"). Linked identifiers.
- Soroban invocation call tree: nested contract-to-contract hierarchy with function names
- Lazy loading: only load/render when section is visible or tab is active. Do not fetch chart data for offscreen sections.

## Implementation Plan

### Step 1: Tab component

Create `libs/ui/src/visualization/Tabs.tsx`:

- Props: tab definitions (label, key), active tab, onChange callback
- Active tab synced to URL query param (e.g., `?tab=invocations`)
- No hard reload on tab change -- React Router search param update
- MUI Tabs as base with explorer-specific styling
- Keyboard accessible: arrow keys navigate tabs, Enter/Space activates

### Step 2: Time-series chart component

Create `libs/ui/src/visualization/TimeSeriesChart.tsx`:

- Props: data points (timestamp + value series), chart type (line/area), title, y-axis label
- Interval selector: 1h, 1d, 1w buttons that trigger data re-fetch
- Date range picker (optional, for custom ranges)
- Responsive: readable on small screens, axes adapt
- Tooltips on hover showing exact values and timestamps
- Lazy rendering: use IntersectionObserver to only render when visible

### Step 3: Operation flow tree component

Create `libs/ui/src/visualization/OperationFlowTree.tsx`:

- Renders transaction operation flow as a graph/tree structure
- Nodes: source account (root) -> operations -> affected accounts/contracts
- Each node displays human-readable summary (e.g., "Sent 1,250 USDC to GD2M...K8J1", "Swapped 100 USDC for 95.2 XLM on Soroswap")
- Identifiers in nodes are linked (using identifier components from task 0062)
- Expandable/collapsible for complex transactions
- Supports both classic operations and Soroban invocations

### Step 4: Soroban invocation call tree

Create `libs/ui/src/visualization/InvocationCallTree.tsx`:

- Renders nested contract-to-contract invocation hierarchy
- Each node shows: contract ID (linked), function name, status
- Nested calls indented to show caller-callee relationship
- Expandable/collapsible for deep call stacks
- Function names displayed prominently, contract IDs as secondary

### Step 5: Lazy loading wrapper

Create `libs/ui/src/visualization/LazySection.tsx`:

- Wraps chart or visualization sections
- Uses IntersectionObserver to detect visibility
- Only renders children (and triggers data fetch) when section enters viewport
- Shows placeholder/skeleton until visible
- Used for chart sections and heavy visualizations on detail pages

### Step 6: Exports

Export all visualization components from `libs/ui` barrel.

## Acceptance Criteria

- [ ] Tabs render with active state synced to URL query params
- [ ] Tab changes do not cause hard page reloads
- [ ] Tabs are keyboard accessible (arrow keys, Enter/Space)
- [ ] TimeSeriesChart renders line/area charts with interval selector (1h/1d/1w)
- [ ] Charts are responsive and readable on small screens
- [ ] Charts show tooltips with exact values on hover
- [ ] OperationFlowTree renders source account -> operations -> affected entities as tree/graph
- [ ] Each tree node shows human-readable summary with linked identifiers
- [ ] InvocationCallTree renders nested contract-to-contract hierarchy with function names
- [ ] Call tree supports expandable/collapsible nodes for deep hierarchies
- [ ] LazySection only renders and fetches data when section is visible in viewport
- [ ] All components exported from `libs/ui`

## Notes

- This is the largest effort task in the frontend shared layer due to the variety of visualization types.
- Chart library selection (e.g., Recharts, Nivo, Victory) should be decided during implementation. Prioritize lightweight bundle size and React compatibility.
- The operation flow tree is consumed by transaction detail normal mode (task 0070).
- The invocation call tree is consumed by both transaction detail (task 0070) and contract detail (task 0075).
- Time-series charts are consumed by liquidity pool detail (task 0077).
