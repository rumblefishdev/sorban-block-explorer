---
id: '0071'
title: 'Frontend: Transaction detail -- advanced mode'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0070']
tags: [priority-high, effort-medium, layer-frontend-pages]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Transaction detail -- advanced mode

## Summary

Implement the advanced mode view for the Transaction detail page (`/transactions/:hash`). Advanced mode targets developers and experienced users, showing per-operation raw parameters, full argument values, events, diagnostic events, and collapsible XDR sections. It preserves exactness and never hides null, empty, or zero values.

## Status: Backlog

**Current state:** Not started.

## Context

Advanced mode is the developer-facing view of a transaction. It uses the same `GET /transactions/:hash` API response as normal mode (task 0070) but presents operations and events with full technical detail. This is critical for debugging smart contract calls and verifying exact transaction parameters.

### API Endpoint Consumed

| Endpoint                  | Purpose                                  |
| ------------------------- | ---------------------------------------- |
| `GET /transactions/:hash` | Same response as normal mode (task 0070) |

### Advanced Mode: Per-Operation Detail

For each operation in the transaction:

| Field           | Display                    | Notes                                   |
| --------------- | -------------------------- | --------------------------------------- |
| Operation ID    | Full value                 | Unique identifier for the operation     |
| Operation Type  | Raw protocol type          | e.g., "invoke_host_function", "payment" |
| Raw Parameters  | Full key-value listing     | All parameters with their exact values  |
| Argument Values | Full values, not truncated | Complete argument data                  |
| Return Values   | Full return data           | For Soroban invocations                 |

CRITICAL: Never hide null, empty, or zero values. Exactness matters for debugging. Show every field as returned by the API.

### Events Section

| Field      | Display               | Notes                                    |
| ---------- | --------------------- | ---------------------------------------- |
| Event Type | Label                 | e.g., "contract", "system", "diagnostic" |
| Topics     | Array of topic values | Full values displayed                    |
| Data       | Event data payload    | Full value, expandable if large          |

- Contract events and diagnostic events displayed in separate sub-sections
- Diagnostic events clearly labeled to distinguish from contract events

### Collapsible XDR Sections

| Section      | Default State | Notes                            |
| ------------ | ------------- | -------------------------------- |
| envelope_xdr | Collapsed     | Full XDR string with copy button |
| result_xdr   | Collapsed     | Full XDR string with copy button |

- Collapsible to keep screen usable when XDR is large
- Copy button on each section for easy clipboard access
- Monospace font for XDR display

## Implementation Plan

### Step 1: Advanced mode operation list

Create `apps/web/src/pages/transaction-detail/AdvancedOperationList.tsx`:

- Renders each operation with full detail
- Shows: operation ID, type, all raw parameters, argument values, return values
- Never hides null/empty/zero values
- Each operation in a distinct card or bordered section

### Step 2: Events section

Create `apps/web/src/pages/transaction-detail/EventsSection.tsx`:

- Lists all events emitted by the transaction
- For each event: type, topics (array), data
- Contract events and diagnostic events in separate sub-sections with headers
- Expandable data fields for large payloads

### Step 3: Collapsible XDR sections

Create `apps/web/src/pages/transaction-detail/XdrSection.tsx`:

- Collapsible panel for `envelope_xdr` and `result_xdr`
- Default state: collapsed
- Expand/collapse toggle with clear label
- Monospace font, horizontal scroll for long lines
- Copy button for each XDR section

### Step 4: Advanced mode view composition

Create `apps/web/src/pages/transaction-detail/AdvancedModeView.tsx`:

- Composes: AdvancedOperationList, EventsSection, XdrSection (envelope), XdrSection (result)
- Each section wrapped in SectionErrorBoundary (task 0064)
- Shown when mode toggle (from task 0070) is set to "Advanced"

## Acceptance Criteria

- [ ] Per-operation detail shows: operation ID, raw type, all parameters, argument values, return values
- [ ] Null, empty, and zero values are NEVER hidden -- all fields displayed as returned
- [ ] Events section shows: type, topics, data for each event
- [ ] Diagnostic events separated from contract events with clear labels
- [ ] envelope_xdr section: collapsible, default collapsed, copy button, monospace font
- [ ] result_xdr section: collapsible, default collapsed, copy button, monospace font
- [ ] Advanced mode uses same API response as normal mode (no separate fetch)
- [ ] Large data payloads are expandable without breaking page layout
- [ ] Each section has independent error boundary

## Notes

- This task depends on task 0070 for the base transaction detail page structure, mode toggle, and shared API response.
- The "never hide values" rule is the key differentiator from normal mode. Developers need to verify exact protocol-level data.
- XDR sections can be very large. Horizontal scroll and collapse-by-default are essential for usability.
