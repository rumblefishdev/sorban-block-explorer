---
id: '0062'
title: 'XDR parsing: Soroban events, invocation tree, contract interface extraction'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0060', '0018', '0003']
tags: [priority-high, effort-large, layer-indexing]
links:
  - docs/architecture/xdr-parsing/xdr-parsing-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# XDR parsing: Soroban events, invocation tree, contract interface extraction

## Summary

Implement Soroban-specific parsing covering three areas: (1) CAP-67 event extraction and persistence to the partitioned soroban_events table, (2) contract invocation tree decoding from result_meta_xdr producing both flat soroban_invocations rows and nested operation_tree JSONB on the transaction, and (3) contract interface extraction from WASM at deployment time into soroban_contracts.metadata. This task also emits parsed NFT-related events for consumption by task 0063.

## Status: Backlog

**Current state:** Not started. Depends on task 0060 for parsed transaction data. Research task 0003 (Soroban WASM interface extraction) and research task 0002 (LedgerCloseMeta parsing) provide foundational knowledge. Database schema task 0018 defines the target tables.

## Context

Soroban transactions produce three categories of data that require specialized parsing beyond basic operation extraction:

1. **CAP-67 Events**: Contract, system, and diagnostic events emitted during transaction execution. These are the primary mechanism for observing contract activity and are critical for the Event Interpreter (task 0067) and explorer event views.

2. **Invocation Trees**: Complex Soroban transactions may involve nested contract-to-contract calls. The full hierarchy must be decoded to support the transaction detail tree view. The explorer needs both flat rows (for querying by contract/function) and nested JSON (for rendering the call tree).

3. **Contract Interfaces**: When a contract is deployed, its WASM binary is available in LedgerEntryChanges. Public function signatures (names, parameter types, return types) can be extracted and stored as metadata for contract detail pages.

### Partitioning

- `soroban_events` is partitioned monthly by `created_at`
- `soroban_invocations` is partitioned monthly by `created_at`

All inserts must target the correct partition.

### Ownership Boundaries

- This task writes soroban_events rows, soroban_invocations rows, and transactions.operation_tree JSONB
- This task writes contract interface data to soroban_contracts.metadata JSONB only
- Contract creation (contract_id, wasm_hash, deployer) is owned by task 0063
- Event interpretation (human-readable summaries) is owned by task 0067

### Source Code Location

- `apps/indexer/src/parsers/soroban/`

## Implementation Plan

### Step 1: CAP-67 Event Extraction

From SorobanTransactionMeta.events, extract per event:

- `eventType`: one of 'contract', 'system', 'diagnostic'
- `contractId`: the contract that emitted the event
- `topics`: array of ScVal-decoded topic values (ScVal[])
- `data`: ScVal-decoded event data payload

Persist each event to the soroban_events table with:

- `transaction_id` FK (from parent transaction surrogate id)
- `contract_id` FK (references soroban_contracts, may be null if contract not yet registered)
- `ledger_sequence` (from parent ledger)
- `created_at` (from parent ledger closeTime, used for monthly partitioning)

ScVal decoding uses the shared library from task 0013 to convert to typed representations (integer, string, address, bytes, map, list).

### Step 2: Invocation Tree Decoding

Decode the full contract-to-contract invocation hierarchy from `result_meta_xdr`. This involves parsing the SorobanTransactionMeta to extract the invocation tree structure.

Produce TWO outputs:

**(a) Flat soroban_invocations rows:**
For each invocation node in the tree:

- `transaction_id` FK
- `contract_id`: the invoked contract
- `caller_account`: the account or contract that initiated this call
- `function_name`: the invoked function
- `function_args`: ScVal-decoded arguments (JSONB)
- `return_value`: ScVal-decoded return value (JSONB)
- `successful`: boolean indicating if this invocation succeeded
- `ledger_sequence`: from parent ledger
- `created_at`: from parent ledger closeTime (used for monthly partitioning)

**(b) Nested hierarchy as transactions.operation_tree JSONB:**
Build a JSON tree structure representing the full call graph:

- Root invocation with children
- Each node contains: contractId, functionName, args, returnValue, successful, and nested children
- This is stored on the transaction row for direct rendering by the transaction detail page

### Step 3: Contract Interface Extraction

At deployment time, when LedgerEntryChanges contain new contract WASM:

- Extract the WASM binary from the ledger entry
- Parse the WASM to identify public function signatures: function names, parameter types, return types
- Store extracted interface data in `soroban_contracts.metadata` JSONB

Important boundary: this step populates metadata only. The contract creation record (contract_id, wasm_hash, deployer_account, deployed_at_ledger, contract_type, is_sac) is owned by task 0063. If the contract row does not yet exist when interface extraction runs, the metadata should be staged for upsert when the contract row is created.

### Step 4: NFT Event Emission

Identify and emit parsed NFT-related events for task 0063 to consume for NFT state derivation. Known patterns include:

- NFT mint events
- NFT transfer events
- NFT metadata update events

The specific event signatures depend on the NFT contract conventions documented in research task 0005. This step produces intermediate data that task 0063 uses to populate/update the nfts table.

## Acceptance Criteria

- [ ] CAP-67 events are extracted with eventType, contractId, topics (ScVal decoded), and data (ScVal decoded)
- [ ] Events are persisted to soroban_events with correct transaction_id FK, ledger_sequence, and created_at
- [ ] soroban_events inserts target the correct monthly partition based on created_at
- [ ] Invocation tree is decoded from result_meta_xdr into both flat rows and nested JSON
- [ ] Flat soroban_invocations rows include transaction_id, contract_id, caller_account, function_name, function_args, return_value, successful, ledger_sequence, created_at
- [ ] soroban_invocations inserts target the correct monthly partition based on created_at
- [ ] transactions.operation_tree JSONB contains the full nested invocation hierarchy
- [ ] Contract interface extraction parses WASM for public function signatures (names, param types, return types)
- [ ] Extracted interface is stored in soroban_contracts.metadata JSONB
- [ ] NFT-related events are identified and emitted for consumption by task 0063
- [ ] ON DELETE CASCADE from transactions properly cleans up soroban_events and soroban_invocations rows
- [ ] Unit tests cover event extraction, invocation tree decoding (including nested calls), interface extraction, and NFT event identification
- [ ] Transactions with Soroban invocations preserve both operation_tree JSONB AND result_meta_xdr

## Notes

- The invocation tree can be deep for complex DeFi transactions involving multiple contract calls. The parser must handle arbitrary nesting depth without stack overflow.
- Diagnostic events may be suppressed in some protocol configurations. The parser should handle their absence gracefully.
- Contract interface extraction quality depends on WASM structure. Not all contracts will have cleanly extractable interfaces. Store what is available; do not fail on partial extraction.
- The GIN index on soroban_events.topics supports topic-based event queries used by the Event Interpreter (task 0067).
