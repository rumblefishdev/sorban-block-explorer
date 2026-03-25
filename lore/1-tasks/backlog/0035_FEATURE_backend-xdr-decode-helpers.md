---
id: '0035'
title: 'Backend: API-time XDR decode helpers for advanced transaction view'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0013', '0023']
tags: [layer-backend, xdr, decode, advanced-view]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: API-time XDR decode helpers for advanced transaction view

## Summary

Implement narrow, on-demand XDR decode helpers in the API layer for advanced transaction views. These helpers decode stored raw payloads (envelope_xdr, result_xdr, result_meta_xdr) using `@stellar/stellar-sdk`. The result_meta_xdr is decoded server-side only (never sent to frontend) for validating or regenerating the operation_tree. This must remain a narrow secondary path, not the primary source of transaction data.

## Status: Backlog

**Current state:** Not started. Depends on tasks 0013 (shared XDR utilities lib) and 0023 (NestJS API bootstrap).

## Context

The API retains a limited decode role for advanced views. At request time, it can decode stored raw payloads when the advanced transaction view or validation/debug paths need fields not in the standard stored read model. This is the secondary parsing path; the primary parsing happens at ingestion time.

### API Specification

**Location:** `apps/api/src/xdr/`

### Decode Targets

| Raw Payload       | Decode Purpose                                        | Returned to Frontend |
| ----------------- | ----------------------------------------------------- | -------------------- |
| `envelope_xdr`    | Advanced transaction inspection                       | Yes (as raw string)  |
| `result_xdr`      | Advanced transaction inspection                       | Yes (as raw string)  |
| `result_meta_xdr` | Server-side validation/regeneration of operation_tree | No, never            |

### Decode Operations

**Envelope XDR decode:**

- Parse `TransactionEnvelope` from stored XDR
- Extract detailed operation parameters for advanced view
- Provide raw parameter objects for individual operations

**Result XDR decode:**

- Parse `TransactionResult` from stored XDR
- Extract detailed result codes and per-operation results
- Provide raw result objects for advanced debugging

**Result Meta XDR decode:**

- Parse `TransactionMeta` from stored XDR
- Extract/validate invocation hierarchy (operation_tree)
- Used for validation of stored operation_tree or regeneration if needed
- NEVER returned to frontend

### Response Integration

The XDR decode helpers are consumed by the Transactions module (task 0027) for the `?view=advanced` detail endpoint. They are NOT standalone endpoints.

**Example usage in advanced view:**

```json
{
  "operations": [
    {
      "type": "invoke_host_function",
      "contract_id": "CCAB...DEF",
      "function_name": "swap",
      "human_readable": "Swapped 100 USDC for 95.2 XLM",
      "raw_parameters": {
        "host_function": "InvokeContract",
        "args": [...]
      }
    }
  ],
  "envelope_xdr": "AAAAAA...",
  "result_xdr": "AAAAAA..."
}
```

### Behavioral Requirements

- CONSTRAINT: Must remain narrow, NOT the primary source of transaction data
- Normal data comes from pre-materialized DB fields (operations, operation_tree, events)
- XDR decode only activates for advanced views or validation paths
- Uses shared lib from task 0013 (libs/shared XDR utilities)
- Graceful failure: if decode fails, return stored raw XDR strings without decoded fields
- Do not block the response if decode fails; degrade to raw-only

### Caching

- No caching at the decode helper level. Transaction detail caching is handled at API Gateway (long TTL for finalized transactions).

### Error Handling

- Decode failures are non-fatal: the response still returns with available data
- If envelope_xdr or result_xdr decode fails, raw strings are still returned
- If result_meta_xdr decode fails, the stored operation_tree from DB is used as-is
- Log decode errors for operational visibility

## Implementation Plan

### Step 1: XDR Service Scaffolding

Create `apps/api/src/xdr/` with XDR decode service.

### Step 2: Envelope Decoder

Implement envelope_xdr decode using `@stellar/stellar-sdk` via the shared lib from task 0013. Extract detailed operation parameters.

### Step 3: Result Decoder

Implement result_xdr decode. Extract detailed result codes and per-operation results.

### Step 4: Result Meta Decoder

Implement result_meta_xdr decode for server-side operation_tree validation/regeneration. Ensure this output is never included in API responses.

### Step 5: Integration with Transactions Module

Expose decode methods for consumption by the Transactions service when `?view=advanced` is requested.

### Step 6: Graceful Failure Handling

Ensure all decode operations fail gracefully. Log errors, return available data without decoded supplement.

## Acceptance Criteria

- [ ] Envelope XDR decode extracts detailed operation parameters
- [ ] Result XDR decode extracts detailed result codes
- [ ] Result meta XDR decode validates/regenerates operation_tree server-side
- [ ] result_meta_xdr NEVER returned to frontend
- [ ] Decode helpers consumed by Transactions module for advanced view
- [ ] Graceful failure: decode errors do not block response
- [ ] Uses shared XDR lib from task 0013
- [ ] Narrow scope: not the primary data source for normal views
- [ ] Decode errors logged for operational visibility
- [ ] No standalone XDR decode endpoints exposed

## Notes

- This is explicitly a narrow, secondary decode path. The primary source of truth is pre-materialized data from ingestion.
- The shared lib from task 0013 provides the low-level `@stellar/stellar-sdk` wrappers; this task focuses on API-layer integration.
- result_meta_xdr decode is useful for validating that the stored operation_tree matches what the XDR would produce, but it should not be run on every request.
