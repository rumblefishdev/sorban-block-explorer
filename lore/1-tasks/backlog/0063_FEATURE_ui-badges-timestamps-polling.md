---
id: '0063'
title: 'UI lib: badges, relative timestamps, polling indicator'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-small, layer-frontend-shared]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# UI lib: badges, relative timestamps, polling indicator

## Summary

Implement badge components, relative timestamp display, and a polling indicator in `libs/ui/src/badges/` and `libs/ui/src/timestamps/`. These small but ubiquitous primitives appear on nearly every page and must be accessible, consistent, and informative.

## Status: Backlog

**Current state:** Not started.

## Context

Badges and timestamps are among the most frequently rendered elements in the explorer. They communicate transaction status, entity type, network environment, and data freshness at a glance. Accessibility is a hard requirement: badges must use visible TEXT labels (not color-only), and timestamps must have sufficient contrast per WCAG guidelines.

Badge types needed:

- Status badges: success / failed (for transactions)
- Type badges: classic / SAC / soroban (for tokens and contracts)
- Network badge: mainnet / testnet variant

Timestamp requirements:

- Relative display: "2 min ago", "1 hour ago", "3 days ago"
- Full ISO timestamp on hover
- Sufficient contrast for secondary metadata per WCAG

Polling indicator:

- Shows "Updated 5s ago" or similar on polling-enabled pages
- Visible on pages with auto-refresh (home, possibly list pages)

## Implementation Plan

### Step 1: Status badge component

Create `libs/ui/src/badges/StatusBadge.tsx`:

- Props: `status` ("success" | "failed")
- Renders colored chip/badge with TEXT label ("Success", "Failed")
- Color: green-toned for success, red-toned for failed -- but text label is the primary indicator, not color alone
- Compact size for table rows

### Step 2: Type badge component

Create `libs/ui/src/badges/TypeBadge.tsx`:

- Props: `type` ("classic" | "sac" | "soroban")
- Renders badge with text label and distinct visual treatment per type
- Used on token list/detail pages and contract pages
- Prevents confusion between similarly named assets of different types

### Step 3: Network badge component

Create `libs/ui/src/badges/NetworkBadge.tsx`:

- Props: `network` ("mainnet" | "testnet")
- Distinct palette per network (from MUI theme in task 0058)
- Used in header network indicator (task 0059) and wherever network context is shown

### Step 4: Relative timestamp component

Create `libs/ui/src/timestamps/RelativeTimestamp.tsx`:

- Props: `timestamp` (ISO string or Date)
- Renders relative time: "2 min ago", "1 hour ago", etc.
- Full ISO timestamp shown on hover via tooltip
- Updates periodically (e.g., every 30s) to keep relative time accurate
- Sufficient contrast ratio per WCAG for secondary metadata text

### Step 5: Polling indicator component

Create `libs/ui/src/timestamps/PollingIndicator.tsx`:

- Props: `lastUpdated` (timestamp), `intervalMs` (polling interval)
- Renders "Updated 5s ago" with a subtle refresh icon
- Visible on polling-enabled pages (home, list pages)
- Updates relative time display periodically

### Step 6: Exports

Export all badge and timestamp components from `libs/ui` barrel.

## Acceptance Criteria

- [ ] StatusBadge renders "Success" or "Failed" with text label (not color-only)
- [ ] TypeBadge renders "Classic", "SAC", or "Soroban" with distinct visual treatment
- [ ] NetworkBadge renders "Mainnet" or "Testnet" with distinct palette
- [ ] RelativeTimestamp shows relative time ("2 min ago") with full ISO on hover
- [ ] Timestamps have sufficient contrast per WCAG guidelines
- [ ] Relative timestamps update periodically to stay accurate
- [ ] PollingIndicator shows "Updated Xs ago" on polling-enabled pages
- [ ] All badges use visible text labels as primary indicator, not color alone
- [ ] All components exported from `libs/ui`

## Notes

- Badge color palette comes from MUI theme configuration in task 0058.
- Status badges are used heavily in transaction tables (tasks 0068, 0069, 0070).
- Type badges are critical for the tokens list/detail (task 0074) and contract detail (task 0075).
- Relative timestamps appear in every table that shows time data.
