---
id: '0070'
title: 'Frontend: Transaction detail -- normal mode'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-large, layer-frontend-pages]
milestone: 2
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: Transaction detail -- normal mode

## Summary

Implement the Transaction detail page (`/transactions/:hash`) with base transaction information and normal mode operation visualization. Normal mode presents a human-readable graph/tree of the transaction's operation flow, designed for general users exploring transactions.

## Status: Backlog

**Current state:** Not started.

## Context

The transaction detail page is the most complex page in the explorer. It has two display modes (normal and advanced) that are alternate presentations over the same backend resource. This task covers the base transaction information and the normal mode. Advanced mode is task 0071.

### API Endpoint Consumed

| Endpoint                  | Purpose                                                           |
| ------------------------- | ----------------------------------------------------------------- |
| `GET /transactions/:hash` | Full transaction detail (supports both normal and advanced modes) |

### Response Fields Used

```json
{
  "hash": "7b2a8c...",
  "ledger_sequence": 12345678,
  "source_account": "GABC...XYZ",
  "successful": true,
  "fee_charged": 100,
  "memo_type": "text",
  "memo": "payment for services",
  "created_at": "2026-03-24T12:00:00Z",
  "operations": [
    {
      "type": "invoke_host_function",
      "contract_id": "CCAB...DEF",
      "function_name": "swap",
      "source_account": "GABC...XYZ",
      "affected_accounts": ["GD2M...K8J1"],
      "affected_contracts": ["CCAB...DEF"]
    }
  ],
  "operation_tree": [...],
  "signatures": [
    {
      "signer": "GABC...XYZ",
      "weight": 1,
      "signature_hex": "a1b2c3..."
    }
  ],
  "envelope_xdr": "...",
  "result_xdr": "..."
}
```

### Base Transaction Information (both modes)

| Field           | Display                              | Notes                              |
| --------------- | ------------------------------------ | ---------------------------------- |
| Hash            | Full, copyable                       | IdentifierWithCopy (task 0062)     |
| Status          | Badge (success/failed)               | StatusBadge (task 0063)            |
| Ledger Sequence | Linked to `/ledgers/:sequence`       | IdentifierDisplay (task 0062)      |
| Timestamp       | Full datetime + relative             | RelativeTimestamp (task 0063)      |
| Fee Charged     | XLM amount + stroops                 | e.g., "0.00001 XLM (100 stroops)"  |
| Source Account  | Full, linked to `/accounts/:id`      | IdentifierWithCopy (task 0062)     |
| Memo            | Type label + content                 | e.g., "Text: payment for services" |
| Signatures      | Table: signer, weight, signature hex | Signer linked to `/accounts/:id`   |

### Normal Mode: Operation Flow

- Graph/tree representation of the transaction's operation flow
- Root node: source account
- Child nodes: operations, each with human-readable summary
- Leaf nodes: affected accounts and contracts
- Node summaries: "Sent 1,250 USDC to GD2M...K8J1", "Swapped 100 USDC for 95.2 XLM on Soroswap"
- All identifiers in nodes are linked
- Soroban invocations: nested call tree showing contract-to-contract hierarchy with function names
- Designed for clarity over completeness -- never expose raw XDR as primary representation

### Mode Toggle

- Prominent toggle between "Normal" and "Advanced" modes
- Toggle preserves page context (does not re-fetch data)
- Both modes use the same API response
- Default mode: Normal

## Implementation Plan

### Step 1: Transaction detail query hook

Create `apps/web/src/pages/transaction-detail/useTransactionDetail.ts`:

- Fetches `GET /transactions/:hash`
- Stale time: 5 minutes (immutable once indexed)
- Query key: `['transactions', hash]`
- Param validation: 64-character hex (from task 0067)

### Step 2: Base transaction info section

Create `apps/web/src/pages/transaction-detail/TransactionInfo.tsx`:

- Renders: hash (full, copyable), status badge, ledger sequence (linked), timestamp, fee (XLM + stroops), source account (linked), memo (type + content)
- Uses identifier components (task 0062), badges (task 0063), timestamps (task 0063)

### Step 3: Signatures section

Create `apps/web/src/pages/transaction-detail/SignaturesTable.tsx`:

- Table: signer (linked), weight, signature hex (truncated, copyable)
- Collapsible if many signatures

### Step 4: Mode toggle

Create `apps/web/src/pages/transaction-detail/ModeToggle.tsx`:

- Prominent button group or segmented control: "Normal" | "Advanced"
- State stored in URL query param (`?mode=normal` or `?mode=advanced`)
- Does not trigger data re-fetch

### Step 5: Normal mode operation flow

Create `apps/web/src/pages/transaction-detail/NormalModeView.tsx`:

- Uses OperationFlowTree component (task 0065)
- Renders operation_tree data as graph/tree
- Each node: human-readable summary with linked identifiers
- Soroban invocations: uses InvocationCallTree (task 0065) for nested contract calls
- Expandable/collapsible for complex transactions

### Step 6: Page composition

Create `apps/web/src/pages/transaction-detail/TransactionDetailPage.tsx`:

- Composes: TransactionInfo, ModeToggle, NormalModeView (or AdvancedModeView from task 0071)
- Each section wrapped in SectionErrorBoundary (task 0064)
- Loading skeleton during fetch
- 404 state: "Transaction not found"

## Acceptance Criteria

- [ ] Base info displays: hash (full, copyable), status badge, ledger sequence (linked), timestamp, fee (XLM + stroops), source account (linked), memo (type + content)
- [ ] Signatures table shows: signer (linked), weight, signature hex
- [ ] Mode toggle is prominent and switches between Normal and Advanced without re-fetching
- [ ] Mode stored in URL query param
- [ ] Normal mode renders operation flow tree with human-readable summaries
- [ ] Each tree node shows linked identifiers
- [ ] Soroban invocations render as nested call tree with function names
- [ ] Normal mode prioritizes clarity -- no raw XDR shown
- [ ] 404 state: "Transaction not found" for invalid/missing hashes
- [ ] Loading skeleton during initial fetch
- [ ] Param validation: rejects non-64-char-hex hashes

## Notes

- Advanced mode is task 0071 and shares the same base info and API response.
- The operation flow tree and invocation call tree components come from task 0065.
- Fee display should show both XLM and stroops for precision (e.g., "0.00001 XLM (100 stroops)").
