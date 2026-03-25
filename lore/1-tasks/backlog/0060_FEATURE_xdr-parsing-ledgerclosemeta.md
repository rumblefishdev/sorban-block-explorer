---
id: '0060'
title: 'XDR parsing: LedgerCloseMeta deserialization, ledger and transaction extraction'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0002', '0013', '0016']
tags: [priority-high, effort-large, layer-indexing]
links:
  - docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
  - docs/architecture/xdr-parsing/xdr-parsing-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# XDR parsing: LedgerCloseMeta deserialization, ledger and transaction extraction

## Summary

Implement the primary XDR parsing entry point in the Ledger Processor that deserializes LedgerCloseMeta payloads, extracts ledger header fields and transaction-level structured data, retains raw XDR artifacts, and persists both ledger and transaction rows to PostgreSQL. This is the foundational parsing task upon which all downstream extraction (operations, events, invocations, entry changes) depends.

## Status: Backlog

**Current state:** Not started. Architecture docs and database schema design are complete. Research task 0002 (LedgerCloseMeta XDR parsing) provides foundational knowledge.

## Context

The block explorer treats LedgerCloseMeta as the canonical input artifact. Every ledger close produces one LedgerCloseMeta payload exported by Galexie to S3 as a zstd-compressed XDR file. The Ledger Processor Lambda must download, decompress, and fully deserialize this payload to populate the explorer's owned PostgreSQL schema.

This task covers the first and most critical parsing stage: turning raw LedgerCloseMeta bytes into structured ledger and transaction rows. All other parsing tasks (0061 operations, 0062 Soroban events/invocations, 0063 entry changes) depend on the output of this stage.

### Design Rationale: Raw XDR Retention

The system deliberately stores both raw XDR and structured data. Raw XDR (envelope_xdr, result_xdr, result_meta_xdr) is retained for advanced inspection, debugging, and protocol-level validation. Structured fields are extracted for fast explorer reads. This is not redundancy -- it is a deliberate architectural tradeoff documented in the XDR parsing overview.

### API-Time XDR Decode Boundary

Narrow, on-demand XDR decode in NestJS for advanced transaction views is NOT part of this task. That capability belongs to the backend API layer. This task covers ingestion-time parsing only.

### Source Code Location

- `apps/indexer/src/parsers/`

### Key Dependencies

- `@stellar/stellar-sdk` for XDR deserialization
- Shared XDR/ScVal parsing library (task 0013)
- Database schema: ledgers table (task 0016), transactions table (task 0016)

## Implementation Plan

### Step 1: S3 Download and Decompression

Implement the S3 object retrieval and zstd decompression pipeline. Input is an S3 key matching pattern `stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd`. Output is raw XDR bytes ready for deserialization.

### Step 2: LedgerCloseMeta Deserialization

Parse the decompressed XDR bytes into a LedgerCloseMeta object using `@stellar/stellar-sdk` XDR types. Handle both LedgerCloseMetaV0 and LedgerCloseMetaV1 (and future versions) as the SDK supports them.

### Step 3: Ledger Header Extraction

Extract from LedgerHeader:

- `sequence` (ledger sequence number)
- `closeTime` (ledger close timestamp)
- `protocolVersion`
- `baseFee`
- `txSetResultHash`
- Ledger `hash` (computed from the ledger header)
- `transaction_count` (number of transactions in this ledger)

Map these to the `ledgers` table schema: sequence (PK), hash (UNIQUE), closed_at, protocol_version, transaction_count, base_fee.

### Step 4: Transaction Extraction

For each transaction in the ledger, extract from TransactionEnvelope and TransactionResult:

- `hash`: SHA-256 of the envelope XDR bytes
- `sourceAccount`: the transaction source account
- `feeCharged`: actual fee charged
- `successful`: boolean success status
- `resultCode`: transaction result code string
- `memo_type`: memo type (none, text, id, hash, return)
- `memo`: memo value if present

### Step 5: Raw XDR Retention

For each transaction, retain raw XDR payloads:

- `envelope_xdr` (NOT NULL) -- the full transaction envelope
- `result_xdr` (NOT NULL) -- the transaction result
- `result_meta_xdr` (nullable) -- the transaction result metadata, when available

These are stored as-is for advanced inspection and debugging.

### Step 6: Timestamp Derivation

Derive `transactions.created_at` from the parent ledger's `closeTime`. All transactions within a ledger share the same created_at timestamp, which is the ledger close time.

### Step 7: Persistence with Surrogate Keys

Persist transaction rows with surrogate `id` (BIGSERIAL) as the primary key. This surrogate key is used by child tables (operations, invocations, events) as their foreign key reference. The `hash` column remains the public lookup key.

Write ledger row first, then all transaction rows for that ledger, within the same database transaction (atomicity handled by task 0064).

### Step 8: Error Handling for Malformed XDR

When `fromXDR()` throws during deserialization:

- Log the error with full transaction context (ledger sequence, transaction index, raw bytes length)
- Store raw XDR verbatim in the transaction row
- Set `parse_error = true` on the transaction record
- Keep the transaction visible in the explorer with all non-XDR fields that are still available
- Do NOT drop or hide malformed transactions

## Acceptance Criteria

- [ ] S3 object download and zstd decompression produces valid XDR bytes
- [ ] LedgerCloseMeta is deserialized using @stellar/stellar-sdk
- [ ] Ledger header fields (sequence, hash, closeTime, protocolVersion, baseFee, txSetResultHash, transaction_count) are correctly extracted and mapped to the ledgers table
- [ ] Transaction fields (hash, sourceAccount, feeCharged, successful, resultCode, memo_type, memo) are correctly extracted per transaction
- [ ] Transaction hash is computed as SHA-256 of envelope XDR bytes
- [ ] Raw XDR fields (envelope_xdr NOT NULL, result_xdr NOT NULL, result_meta_xdr nullable) are retained per transaction
- [ ] transactions.created_at is derived from the parent ledger closeTime
- [ ] Transactions use BIGSERIAL surrogate id for child FK references
- [ ] Malformed XDR triggers error logging with context, raw XDR storage, parse_error=true flag, and the transaction remains visible
- [ ] Unit tests cover ledger header extraction, transaction extraction, hash computation, and error handling paths
- [ ] Parser output is consumable by downstream tasks (0061, 0062, 0063) without re-parsing the LedgerCloseMeta
- [ ] S3 object key pattern validated: stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd
- [ ] zstd decompression handles the documented file format
- [ ] Verify whether txSetResultHash from LedgerHeader is needed; if so, add to ledger extraction

## Notes

- The operations table is partitioned by transaction_id, so the surrogate id assigned here must be available before operation insertion (task 0061).
- Protocol upgrades that change LedgerCloseMeta structure are handled by updating @stellar/stellar-sdk. These are infrequent and announced in advance.
- This parser must be deterministic: the same LedgerCloseMeta input must always produce the same output rows, supporting idempotent replay (task 0065).
- The parser does NOT write to derived-state tables (accounts, tokens, nfts, pools). Those are handled by task 0063.
