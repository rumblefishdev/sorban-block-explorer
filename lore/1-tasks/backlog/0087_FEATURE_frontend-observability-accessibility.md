---
id: '0087'
title: 'Frontend: observability and accessibility baseline'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-medium, effort-medium, layer-frontend-pages]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: observability and accessibility baseline

## Summary

Establish the observability instrumentation and accessibility baseline across the explorer frontend. Covers error boundaries with reporting, failed API tracking, performance timing, and accessibility requirements including keyboard navigation, semantic HTML, and contrast standards.

## Status: Backlog

**Current state:** Not started.

## Context

Observability must provide operational signals to help maintain the product without substituting for backend correctness. Accessibility ensures the explorer is usable without relying on hover-only or color-only cues. Both are cross-cutting concerns applied across all pages.

### Error Boundaries

- Route-level error boundaries (from task 0067) must report:
  - Route path where the error occurred
  - Error message and stack trace
  - Component stack (React component hierarchy)
- Reports sent to observability backend (e.g., Sentry, CloudWatch RUM, or custom endpoint)

### Failed API Request Tracking

For every failed API request, log:

| Field         | Description                                                        |
| ------------- | ------------------------------------------------------------------ |
| Endpoint      | API path called (e.g., `/transactions/abc123`)                     |
| Status Code   | HTTP status (404, 429, 500, etc.)                                  |
| Response Time | Duration of the request in ms                                      |
| Error Type    | Classification: not_found, rate_limit, server_error, network_error |

- Distinguish 404 (not found) from 5xx (server error) from network failures
- Track at the TanStack Query error handler level (task 0066)

### Performance Timing

Measure and report rendering performance for:

| Target                  | What to Measure                                    |
| ----------------------- | -------------------------------------------------- |
| Home page               | Time to interactive, time to first data render     |
| Transaction detail page | Time to interactive, time to operation tree render |
| Contract detail page    | Time to interactive, time to interface tab render  |
| Charts (LP detail)      | Time to chart render after data arrives            |
| Operation tree          | Render time for complex transaction trees          |

- Use browser Performance API (performance.mark / performance.measure)
- Report as operational signals, not analytics

### Keyboard Accessibility

- Full tab-order through: navigation links, search bar, table rows/links, pagination controls, tabs, copy buttons
- Focus management on route transitions: focus moves to main content area or page heading
- Escape closes dropdowns (search suggestions, menus)
- Enter/Space activates interactive elements
- Arrow keys navigate tabs and search suggestions

### Semantic HTML Requirements

| Element                              | Usage                                        |
| ------------------------------------ | -------------------------------------------- |
| `<table>`, `<thead>`, `<th>`, `<td>` | All data tables (NOT div grids)              |
| `<h1>` through `<h6>`                | Heading hierarchy per page                   |
| `<nav>`                              | Primary navigation                           |
| `<main>`                             | Main content area                            |
| `<header>`                           | Page header                                  |
| `<button>`                           | All interactive buttons (not clickable divs) |

### Badge Accessibility

- All badges (status, type, network) use visible TEXT labels as primary indicator
- Color is supplementary, never the sole differentiator
- Sufficient color contrast for badge text on badge background

### Contrast Requirements

Ensure sufficient contrast (WCAG AA minimum) for:

- Timestamps and relative time text
- Secondary metadata (fees, counts, protocol versions)
- Disabled states (disabled pagination buttons, inactive tabs)
- Placeholder text in search inputs

## Implementation Plan

### Step 1: Error reporting setup

Create `apps/web/src/observability/errorReporting.ts`:

- Integration with error reporting service (Sentry or equivalent)
- Helper to capture route path, error, component stack
- Wire into route-level error boundaries

### Step 2: API request tracking

Create `apps/web/src/observability/apiTracking.ts`:

- TanStack Query global error handler integration
- Logs: endpoint, status code, response time, error type classification
- Uses error classification utility from task 0064

### Step 3: Performance timing

Create `apps/web/src/observability/performanceTiming.ts`:

- Helpers wrapping Performance API (mark, measure)
- Page-level timing hooks for home, transaction detail, contract detail
- Component-level timing for charts and operation tree
- Reports to observability backend

### Step 4: Focus management

Create `apps/web/src/accessibility/focusManagement.ts`:

- Hook: `useRouteTransitionFocus` -- moves focus to main content or heading on route change
- Utility: `announceToScreenReader` -- live region announcement for dynamic updates

### Step 5: Accessibility audit tooling

Add eslint-plugin-jsx-a11y to lint config:

- Enforce semantic HTML rules
- Flag missing alt text, missing labels, clickable divs
- Run as part of CI lint step

### Step 6: Contrast and badge audit

- Verify badge text contrast meets WCAG AA
- Verify timestamp/secondary text contrast
- Verify disabled state contrast
- Document color contrast ratios in theme (task 0058)

## Acceptance Criteria

- [ ] Route-level error boundaries report: route path, error, component stack to observability service
- [ ] Failed API requests tracked: endpoint, status code, response time, error type
- [ ] 404 distinguished from 5xx distinguished from network errors in tracking
- [ ] Performance timing measured for: home page, transaction detail, contract detail, charts, operation tree
- [ ] Full keyboard tab-order through nav, search, tables, pagination, tabs, copy buttons
- [ ] Focus managed on route transitions (focus moves to content area)
- [ ] Semantic HTML: `<table>` for data, `<h1>`-`<h6>` hierarchy, `<nav>`, `<main>`, `<header>`
- [ ] All badges use text labels, not color-only
- [ ] WCAG AA contrast met for: timestamps, secondary metadata, disabled states
- [ ] eslint-plugin-jsx-a11y integrated in lint config

## Notes

- Observability is for operational signals only. It should help spot UI failures, slow routes, and degraded views. It must never substitute for backend correctness.
- Accessibility is a baseline requirement, not a stretch goal. Semantic HTML and keyboard access are enforced from the start.
- The MUI theme (task 0058) should encode contrast-compliant color choices so individual components inherit correct values.
