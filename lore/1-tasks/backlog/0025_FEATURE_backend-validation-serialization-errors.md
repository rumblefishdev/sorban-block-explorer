---
id: '0025'
title: 'Backend: request validation, response serialization, error mapping'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0014']
tags: [layer-backend, validation, serialization, error-handling]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Backend: request validation, response serialization, error mapping

## Summary

Implement the cross-cutting request validation, response serialization, and error mapping layer for the API. This includes input validation pipes, response shaping rules, error-to-HTTP mapping, parse_error handling for degraded transactions, unknown operation type handling, and graceful degradation when ingestion is behind.

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (NestJS API bootstrap).

## Context

The API must present consistent, frontend-friendly responses while handling edge cases such as parse errors, unknown operation types, and ingestion lag. Validation, serialization, and error handling are cross-cutting concerns used by all modules.

### API Specification

**Location:** `apps/api/src/common/validation/`, `apps/api/src/common/errors/`

### Response Shaping Rules

1. **Flatten/restructure** nested data for client usability
2. **Attach human-readable labels** produced during ingestion
3. **Raw payloads only** for advanced/detail views, never in list responses
4. **Stable identifier fields** for cross-page linking (hash, account_id, contract_id, etc.)

### Error Envelope

All errors use a consistent envelope:

```json
{
  "error": {
    "code": "string",
    "message": "string"
  }
}
```

### Error-to-HTTP Mapping

| HTTP Status | Condition                                                          | Example Code                                          |
| ----------- | ------------------------------------------------------------------ | ----------------------------------------------------- |
| 400         | Validation failures (bad params, invalid cursor, malformed filter) | `VALIDATION_ERROR`, `INVALID_CURSOR`, `INVALID_LIMIT` |
| 404         | Resource not found (unknown hash, account_id, contract_id)         | `NOT_FOUND`                                           |
| 500         | Internal server errors (DB failures, unexpected exceptions)        | `INTERNAL_ERROR`                                      |

### parse_error Handling

Transactions with `parse_error=true` in the database:

- Remain visible in list and detail endpoints
- Non-XDR fields (hash, ledger_sequence, source_account, fee_charged, successful, created_at) served normally
- XDR-derived fields (operations, operation_tree, events) may be null
- Response includes `parse_error: true` indicator so frontend can display appropriate messaging

**Example response for parse_error transaction:**

```json
{
  "hash": "7b2a8c...",
  "ledger_sequence": 12345678,
  "source_account": "GABC...XYZ",
  "successful": true,
  "fee_charged": 100,
  "created_at": "2026-03-20T12:00:00Z",
  "operations": null,
  "operation_tree": null,
  "events": null,
  "parse_error": true
}
```

### Unknown Operation Types

When an operation type is not recognized by the current SDK version:

```json
{
  "type": "unknown",
  "raw_xdr": "AAAAAA..."
}
```

- Never hide the parent transaction because of an unknown operation
- The transaction remains fully visible with the unknown operation rendered inline

### Graceful Degradation

- All endpoints function when ingestion is behind the network tip
- No errors solely due to stale data
- Freshness is communicated via network stats (highest_indexed_ledger, ingestion_lag_seconds)
- Missing recent data simply means it has not been indexed yet, not an error condition

### Caching

- Validation and serialization are stateless; no caching at this layer.

### Error Handling

Input validation errors:

```json
{
  "error": {
    "code": "VALIDATION_ERROR",
    "message": "filter[type] must be one of: classic, sac, soroban"
  }
}
```

Resource not found:

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Transaction with hash 'abc123' not found."
  }
}
```

## Implementation Plan

### Step 1: Input Validation Pipes

Create NestJS validation pipes for common parameter patterns: path params (hash, account_id, contract_id, sequence), query params (limit, cursor), and filter params. Map validation failures to 400 responses with descriptive messages.

### Step 2: Global Exception Filter

Implement (or extend from 0023) the global exception filter that catches all unhandled exceptions and maps them to the error envelope. Database errors map to 500. Known application errors (NotFound, ValidationError) map to appropriate HTTP codes.

### Step 3: Response Serialization Interceptor

Create a NestJS interceptor or serialization layer that applies response shaping rules: flatten nested fields, ensure stable identifiers are present, strip raw payloads from non-advanced responses.

### Step 4: parse_error Handling

Implement a serialization rule that detects `parse_error=true` on transaction records, sets XDR-derived fields to null, and includes the `parse_error` indicator in the response.

### Step 5: Unknown Operation Type Handling

Implement fallback serialization for unrecognized operation types, rendering them as `{ type: 'unknown', raw_xdr: '...' }` without hiding the parent transaction.

### Step 6: Graceful Degradation Verification

Ensure no endpoint throws errors solely because ingestion is behind. Verify that empty result sets and missing recent data are handled as normal (empty list, 404 for specific missing resource) rather than error conditions.

## Acceptance Criteria

- [ ] Input validation pipes for all common parameter types
- [ ] Consistent error envelope `{ error: { code, message } }` on all error responses
- [ ] 400 for validation failures, 404 for missing resources, 500 for internal errors
- [ ] Response shaping: flatten nested data, attach human-readable labels, stable identifiers
- [ ] Raw payloads excluded from non-advanced responses
- [ ] parse_error transactions visible with available fields, XDR-derived fields null
- [ ] Unknown operations rendered as `{ type: 'unknown', raw_xdr: '...' }`
- [ ] Parent transactions never hidden due to unknown child operations
- [ ] All endpoints function when ingestion is behind (no stale-data errors)
- [ ] Error messages are descriptive and actionable

## Notes

- This task provides shared infrastructure consumed by all feature module tasks (0026-0034).
- The parse_error and unknown operation handling are critical for explorer resilience during protocol upgrades.
- Graceful degradation is a fundamental architectural requirement, not an optional enhancement.
