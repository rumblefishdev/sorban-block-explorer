---
id: '0014'
title: 'Shared error types and parse_error handling'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0013', '0056', '0017']
tags: [priority-medium, effort-small, layer-domain]
links:
  - docs/architecture/xdr-parsing/xdr-parsing-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-27
    status: active
    who: FilipDz
    note: 'Activated task'
---

# Shared error types and parse_error handling

## Summary

Define shared error types and the parse_error handling strategy for the block explorer. These types live in `libs/shared` and establish the error taxonomy for XDR parsing failures, unknown operation types, ScVal decode errors, and contract metadata extraction failures. The core principle is: "log, store raw, mark parse_error, keep visible." Never drop data silently.

## Status: Backlog

**Current state:** Not started. Closely related to the XDR parsing library (task 0013).

## Context

The block explorer ingests canonical Stellar ledger data and decodes it into structured records. Decoding can fail for several reasons: malformed XDR, unsupported operation types from new protocol versions, corrupted ScVal payloads, or WASM extraction failures. The system must handle all of these gracefully without losing data.

The fundamental design principle is that partial records are always preferred over missing records. A transaction with a failed XDR decode should still be visible in the explorer with all non-XDR fields intact.

### Error taxonomy

#### XdrParseError

**Trigger:** `fromXDR()` throws during ingestion.

**Handling:**

- Log the error with full transaction context (hash, ledger sequence, error message)
- Store the raw XDR verbatim in `envelope_xdr`, `result_xdr`, `result_meta_xdr`
- Set `parse_error = true` on the transaction record
- Keep the transaction visible in the explorer with all non-XDR fields that are still available
- The transaction appears in list views and is accessible by hash, but structured decode fields (operations, operation_tree) may be empty or partial

#### UnknownOperationType

**Trigger:** A new protocol version introduces an operation type not yet supported by `@stellar/stellar-sdk`.

**Handling:**

- Render the operation as "unknown" type in explorer responses
- Show raw XDR in the advanced view so users can still inspect the payload
- Log and alarm for SDK update -- this signals that `@stellar/stellar-sdk` needs to be updated
- Do not fail the entire transaction decode; process all other operations normally

#### ScValDecodeError

**Trigger:** A malformed or unexpected ScVal is encountered during decode of function args, return values, or event topics/data.

**Handling:**

- Log the error with context (transaction hash, contract ID, field being decoded)
- Store the raw undecoded value alongside the error marker
- Mark the specific field as unparsed in the JSONB output
- Do not fail the parent invocation or event record; preserve all other decoded fields

#### ContractMetadataError

**Trigger:** WASM interface extraction fails for a deployed contract.

**Handling:**

- Log the error with contract ID and WASM hash
- Store the contract record without interface data (metadata.interface will be absent)
- The contract remains visible and functional in the explorer; only the interface tab will be empty
- Do not block or fail the deployment record ingestion

### Core principle

"Log, store raw, mark parse_error, keep visible."

Every error handler follows the same pattern:

1. Log the error with sufficient context for debugging
2. Store whatever raw data is available verbatim
3. Mark the affected record with an appropriate error flag
4. Keep the record visible in the explorer -- never silently drop it

Partial records are always preferred over missing records.

### Protocol upgrade context

Protocol upgrades that affect `LedgerCloseMeta` structure are:

- Infrequent (a few times per year at most)
- Announced well in advance by the Stellar Development Foundation
- Handled by updating `@stellar/stellar-sdk` to the latest version

The error types defined here provide a safety net for the gap between a protocol upgrade going live and the SDK update being deployed.

## Implementation Plan

### Step 1: Define error base type

Create a base `ExplorerParseError` type in `libs/shared` with common fields: errorType, message, context (transaction hash, ledger sequence, contract ID as applicable), timestamp.

### Step 2: Define XdrParseError type

Create `XdrParseError` extending the base type. Include fields for the raw XDR that failed to decode and the specific decode step that failed.

### Step 3: Define UnknownOperationType error

Create `UnknownOperationType` error type. Include the raw operation type identifier and the raw XDR for the unknown operation.

### Step 4: Define ScValDecodeError type

Create `ScValDecodeError` error type. Include the raw ScVal bytes, the field context (e.g., "functionArgs", "returnValue", "topics"), and the parent record identifier.

### Step 5: Define ContractMetadataError type

Create `ContractMetadataError` error type. Include the contract ID, WASM hash, and the extraction step that failed.

### Step 6: Implement error handler utilities

Create utility functions that implement the "log, store raw, mark" pattern:

- `handleXdrParseError(error, txContext)` -- logs, returns partial record with parse_error=true
- `handleUnknownOperation(opType, rawXdr)` -- logs, returns unknown-typed operation record
- `handleScValDecodeError(error, fieldContext)` -- logs, returns raw-marked field
- `handleContractMetadataError(error, contractContext)` -- logs, returns contract without interface

### Step 7: Export and test

Export all error types and handlers from `libs/shared`. Write unit tests verifying that each handler produces the expected partial record and does not throw.

## Acceptance Criteria

- [ ] `XdrParseError` type defined with raw XDR storage and transaction context
- [ ] `UnknownOperationType` type defined with raw type identifier and XDR
- [ ] `ScValDecodeError` type defined with raw ScVal, field context, and parent identifier
- [ ] `ContractMetadataError` type defined with contract ID and WASM hash
- [ ] Error handler utilities implement "log, store raw, mark" pattern for each error type
- [ ] No error handler silently drops data -- partial records are always produced
- [ ] All error types and handlers exported from `libs/shared`
- [ ] Unit tests verify each handler produces partial records without throwing
- [ ] Types compile without errors
- [ ] UnknownOperationType handler emits a structured log event suitable for CloudWatch alarm triggering

## Notes

- The `parse_error` boolean on the transactions table is the primary DB-level flag for XDR decode failures.
- Unknown operation types should trigger an alarm/notification so the team knows to update `@stellar/stellar-sdk`.
- ScVal decode errors affect JSONB fields on invocations and events; the parent record should remain intact.
- Contract metadata errors only affect the interface tab; all other contract data remains functional.
- Protocol upgrades are infrequent and announced. These error types are a safety net, not a routine flow.
