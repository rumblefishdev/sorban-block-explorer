---
id: '0042'
title: 'UI lib: identifier display, copy button, linked identifiers'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-small, layer-frontend-shared]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# UI lib: identifier display, copy button, linked identifiers

## Summary

Implement identifier display components in `libs/ui/src/identifiers/` that provide consistent rendering, truncation, copy-to-clipboard, and deep linking for all entity identifiers across the explorer. Every hash, account ID, contract ID, token ID, pool ID, and ledger sequence in the app must look and behave identically.

## Status: Backlog

**Current state:** Not started.

## Context

Identifiers are the primary navigation anchors in a block explorer. Users constantly scan, copy, and click identifiers to move between entities. Visual consistency is critical -- the same identifier must look the same whether it appears in a table row, a detail page header, or a search result.

Linkable entity types and their routes:

- Transaction hash: `/transactions/:hash`
- Account ID: `/accounts/:id`
- Contract ID: `/contracts/:id`
- Token ID: `/tokens/:id`
- Pool ID: `/liquidity-pools/:id`
- Ledger sequence: `/ledgers/:seq`

Display requirements:

- Visually identical everywhere: same truncation rules, font, hover behavior, link styling
- Copy confirmation: brief non-intrusive tooltip "Copied!" for 1-2 seconds
- Each component accepts entity type to determine link target
- Dependency on `libs/domain` for identifier format validation utilities

## Implementation Plan

### Step 1: Identifier display component

Create `libs/ui/src/identifiers/IdentifierDisplay.tsx`:

- Props: `value` (full identifier string), `type` (entity type enum), `truncate` (boolean, default true), `linked` (boolean, default true)
- Truncation: shows first N and last M characters with ellipsis (e.g., "GABC...XYZ1")
- Full value shown on hover via tooltip
- Monospace font for all identifiers
- When `linked=true`, renders as a React Router `<Link>` to the appropriate detail page based on `type`

### Step 2: Copy button component

Create `libs/ui/src/identifiers/CopyButton.tsx`:

- Small icon button adjacent to identifier
- On click: copies full identifier value to clipboard
- Shows brief "Copied!" tooltip for 1-2 seconds, then reverts
- Non-intrusive: does not displace layout or obscure content
- Accessible: aria-label "Copy to clipboard", announces copy success

### Step 3: Composed identifier with copy

Create `libs/ui/src/identifiers/IdentifierWithCopy.tsx`:

- Composes `IdentifierDisplay` + `CopyButton` inline
- For full-length display contexts (detail page headers): show full value + copy button
- For table/list contexts: show truncated value + copy on hover/focus

### Step 4: Entity type routing map

Create `libs/ui/src/identifiers/identifierRoutes.ts`:

- Maps entity type enum to route pattern
- Used by `IdentifierDisplay` to generate correct `<Link>` target
- Types: transaction, account, contract, token, pool, ledger, nft

### Step 5: Identifier format validation (libs/domain)

Add or verify identifier format validation utilities in `libs/domain`:

- Transaction hash: 64-character hex
- Account ID: G... format (Stellar public key)
- Contract ID: C... format
- Ledger sequence: positive integer
- Token ID, Pool ID, NFT ID: string validation

### Step 6: Exports

Export all identifier components from `libs/ui` barrel.

## Acceptance Criteria

- [ ] IdentifierDisplay renders with consistent truncation, font, and hover behavior everywhere
- [ ] Truncated identifiers show full value on hover via tooltip
- [ ] Linked identifiers navigate to the correct detail page based on entity type
- [ ] CopyButton copies full value to clipboard and shows "Copied!" confirmation for 1-2 seconds
- [ ] Copy confirmation is non-intrusive (tooltip, not modal or toast)
- [ ] IdentifierWithCopy composes display and copy for both full and truncated contexts
- [ ] Entity type routing map covers: transaction, account, contract, token, pool, ledger, nft
- [ ] Monospace font used for all identifier strings
- [ ] Keyboard accessible: copy button focusable and activatable via Enter/Space
- [ ] All components exported from `libs/ui`

## Notes

- This component set is used by virtually every page in the explorer. Consistency is paramount.
- Identifier format validation in `libs/domain` may already be partially implemented from tasks 0009-0012.
- The truncation algorithm should be configurable but have sensible defaults (e.g., 6 chars + ... + 4 chars).
