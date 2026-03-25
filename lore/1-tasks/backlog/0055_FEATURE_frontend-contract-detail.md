---
id: '0055'
title: 'Frontend: Contract detail page'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-large, layer-frontend-pages]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Contract detail page

## Summary

Implement the Contract detail page (`/contracts/:contractId`) with summary, interface, invocations, events, and stats. This is the primary developer-facing entrypoint for Soroban contracts and the most Soroban-specific page in the explorer.

## Status: Backlog

**Current state:** Not started.

## Context

The contract page must serve as a comprehensive developer tool for understanding a Soroban contract's metadata, public interface, usage patterns, and event history. It uses four separate API endpoints for independent section fetching. SAC (Stellar Asset Contract) identification must be visually clear.

### API Endpoints Consumed (4 endpoints)

| Endpoint                                  | Purpose                                                       |
| ----------------------------------------- | ------------------------------------------------------------- |
| `GET /contracts/:contract_id`             | Contract metadata: ID, deployer, WASM hash, stats, SAC status |
| `GET /contracts/:contract_id/interface`   | Public function signatures: names, param types, return types  |
| `GET /contracts/:contract_id/invocations` | Paginated list of contract invocations                        |
| `GET /contracts/:contract_id/events`      | Paginated list of contract events                             |

### Contract Summary Fields

| Field              | Display                         | Notes                                                                                 |
| ------------------ | ------------------------------- | ------------------------------------------------------------------------------------- |
| Contract ID        | Full, copyable                  | IdentifierWithCopy (task 0042). Prominent at page top.                                |
| Deployer           | Full, linked to `/accounts/:id` | IdentifierWithCopy (task 0042)                                                        |
| Deployed At Ledger | Linked to `/ledgers/:sequence`  | IdentifierDisplay (task 0042)                                                         |
| WASM Hash          | Full, copyable                  | IdentifierWithCopy (task 0042)                                                        |
| SAC Badge          | Badge if applicable             | "Stellar Asset Contract" badge. Visually clear, materially changes user expectations. |
| Total Invocations  | Integer                         | Stats: total invocation count                                                         |
| Unique Callers     | Integer                         | Stats: unique caller count                                                            |

### Interface Tab: Function Signatures

For each public function:

| Field         | Display               | Notes                               |
| ------------- | --------------------- | ----------------------------------- |
| Function Name | Prominent text        | Primary identifier of the function  |
| Parameters    | Name + type per param | e.g., "amount: i128", "to: Address" |
| Return Type   | Type                  | e.g., "bool", "i128"                |

- Readable format, not raw ABI dump
- Should be understandable by non-authors of the contract
- Separate from invocation/event data

### Invocations Tab: Table Columns

| Column        | Display                              | Notes                         |
| ------------- | ------------------------------------ | ----------------------------- |
| Function Name | Text                                 | Which function was called     |
| Caller        | Truncated, linked to `/accounts/:id` | IdentifierDisplay (task 0042) |
| Status        | Badge (success/failed)               | StatusBadge (task 0043)       |
| Ledger        | Linked to `/ledgers/:sequence`       | IdentifierDisplay (task 0042) |
| Timestamp     | Relative                             | RelativeTimestamp (task 0043) |

- Paginated with cursor-based pagination

### Events Tab: Table Columns

| Column     | Display                        | Notes                                           |
| ---------- | ------------------------------ | ----------------------------------------------- |
| Event Type | Label                          | e.g., "contract", "system"                      |
| Topics     | Array display                  | Topic values                                    |
| Data       | Expandable                     | Event data payload, expandable for large values |
| Ledger     | Linked to `/ledgers/:sequence` | IdentifierDisplay (task 0042)                   |

- Paginated with cursor-based pagination
- Include interpretations when available from the backend

## Implementation Plan

### Step 1: Contract detail query hooks

Create `apps/web/src/pages/contract-detail/` with four query hooks:

- `useContractDetail.ts`: fetches `GET /contracts/:contract_id`, stale time 5 minutes
- `useContractInterface.ts`: fetches `GET /contracts/:contract_id/interface`, stale time 5 minutes
- `useContractInvocations.ts`: fetches `GET /contracts/:contract_id/invocations` with cursor
- `useContractEvents.ts`: fetches `GET /contracts/:contract_id/events` with cursor
- All four queries issued independently

### Step 2: Contract summary section

Create `apps/web/src/pages/contract-detail/ContractSummary.tsx`:

- Renders: contract ID (full, copyable), deployer (linked), deployed at ledger (linked), WASM hash (copyable), SAC badge (if applicable)
- Stats: total invocations, unique callers

### Step 3: Interface tab

Create `apps/web/src/pages/contract-detail/ContractInterface.tsx`:

- Lists all public functions
- Each function: name (prominent), parameters (name: type), return type
- Readable layout, not raw ABI

### Step 4: Invocations tab

Create `apps/web/src/pages/contract-detail/ContractInvocations.tsx`:

- Paginated table: function name, caller (linked), status badge, ledger (linked), timestamp
- Uses ExplorerTable (task 0041)

### Step 5: Events tab

Create `apps/web/src/pages/contract-detail/ContractEvents.tsx`:

- Paginated table: event type, topics, data, ledger (linked)
- Expandable data fields for large event payloads
- Interpretations shown when available

### Step 6: Page composition with tabs

Create `apps/web/src/pages/contract-detail/ContractDetailPage.tsx`:

- ContractSummary at top (always visible)
- Tabs (task 0045): Interface, Invocations, Events
- Active tab in URL query param
- Each section in SectionErrorBoundary (task 0044)
- Param validation: C... format (from task 0047)
- 404 state: "Contract not found"

## Acceptance Criteria

- [ ] Summary shows: contract ID (full, copyable), deployer (linked), deployed at ledger (linked), WASM hash (copyable), SAC badge
- [ ] Stats display: total invocations, unique callers
- [ ] Interface tab lists functions with param names, types, and return types in readable format
- [ ] Invocations tab: paginated table with function name, caller (linked), status, ledger, timestamp
- [ ] Events tab: paginated table with event type, topics, data, ledger
- [ ] Tabs do not cause hard reloads; active tab in URL
- [ ] All four API endpoints fetched independently (partial failure isolated)
- [ ] SAC badge visually prominent when applicable
- [ ] Param validation: C... format for contractId
- [ ] 404 state: "Contract not found"
- [ ] Loading skeleton and error states per section

## Notes

- This is the most Soroban-specific page. It must work as a developer tool.
- The interface tab is especially important: it should make contract APIs understandable without reading source code.
- SAC identification materially changes user expectations (it represents a wrapped classic asset, not a custom contract).
- Four independent queries allow the page to render progressively and degrade gracefully.
