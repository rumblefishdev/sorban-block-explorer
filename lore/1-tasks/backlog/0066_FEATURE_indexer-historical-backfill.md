---
id: '0066'
title: 'Indexer: historical backfill Fargate task'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0001', '0064', '0065']
tags: [priority-medium, effort-medium, layer-indexing]
links:
  - docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Indexer: historical backfill Fargate task

## Summary

Implement an ECS Fargate task that reads Stellar public history archives, exports LedgerCloseMeta XDR files to the same S3 bucket used by live Galexie ingestion, and triggers the standard Ledger Processor Lambda for parsing and persistence. This enables the explorer to backfill historical data from Soroban mainnet activation onward while reusing the exact same processing pipeline as live ingestion.

## Status: Backlog

**Current state:** Not started. Depends on the Ledger Processor (task 0064) and idempotent writes (task 0065) for downstream processing. Research task 0001 (Galexie/Captive Core setup) provides foundational knowledge.

## Context

The block explorer needs historical chain data from Soroban mainnet activation (late 2023, approximately ledger 50,692,993) to present day. Live Galexie ingestion handles new ledgers going forward, but the historical gap must be filled by a separate backfill process.

The architecture explicitly avoids a separate parse path for backfill. Instead, the backfill task writes the same XDR file format to the same S3 bucket, which triggers the same Ledger Processor Lambda. This keeps the ingestion contract uniform and eliminates divergence between historical and live processing logic.

Live-derived state remains authoritative for the newest ledgers. Backfill data must not overwrite newer state, which is enforced by the watermark logic in task 0065.

### Source Code Location

- `apps/indexer/src/backfill/`

## Implementation Plan

### Step 1: History Archive Reader

Implement a reader that connects to Stellar public history archives and retrieves LedgerCloseMeta for specified ledger ranges. The reader should:

- Accept configurable start and end ledger sequence numbers
- Default scope: Soroban mainnet activation (~ledger 50,692,993) to the current tip
- Read LedgerCloseMeta payloads from the archive in sequence

### Step 2: S3 Output Writer

For each retrieved LedgerCloseMeta, write to S3 using the same format as Galexie:

- Bucket: `stellar-ledger-data`
- Key pattern: `ledgers/{seq_start}-{seq_end}.xdr.zstd`
- Compression: zstd

The S3 PutObject triggers the same Ledger Processor Lambda (task 0064) via the S3 event notification configured in CDK task 0069.

### Step 3: Configurable Batch Processing

Support configurable ledger-range batches:

- Batch size (number of ledgers per S3 file)
- Rate limiting to avoid overwhelming the Ledger Processor Lambda
- Progress tracking: log current position and estimated completion

### Step 4: Parallel Non-Overlapping Ranges

Support running multiple backfill Fargate tasks in parallel:

- Each task owns a non-overlapping ledger range (e.g., task A: 50M-51M, task B: 51M-52M)
- Ranges are specified via task parameters (start, end)
- Deterministic replay: the same range always produces the same output
- No coordination needed between parallel tasks because ranges do not overlap

### Step 5: Fargate Task Configuration

Configure as an ECS Fargate task (infrastructure in CDK task 0071):

- VPC placement: private subnet
- Outbound: NAT Gateway for Stellar archive access, S3 via VPC endpoint
- Task role: S3 PutObject on stellar-ledger-data, CloudWatch Logs
- Accept ledger range parameters (start, end) via environment variables or task overrides

### Step 6: Safety Guarantees

Ensure backfill cannot corrupt live data:

- Output goes to S3, which triggers the standard Lambda -- no direct database writes
- Watermark logic (task 0065) prevents backfill from overwriting newer live state
- If a backfill file triggers Lambda processing for a ledger already processed by live ingestion, the immutable INSERT ON CONFLICT DO NOTHING handles it safely
- Backfill can be stopped and resumed at any point by adjusting the start ledger

## Acceptance Criteria

- [ ] Fargate task reads LedgerCloseMeta from Stellar public history archives
- [ ] Default scope starts from Soroban mainnet activation (~ledger 50,692,993)
- [ ] Start and end ledger are configurable via parameters
- [ ] Output format matches Galexie: `stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd`
- [ ] S3 PutObject triggers the same Ledger Processor Lambda used by live ingestion
- [ ] Multiple parallel tasks with non-overlapping ranges work correctly
- [ ] No separate parse path -- all processing goes through the standard Ledger Processor
- [ ] Live-derived state is NOT overwritten by backfill (enforced by task 0065 watermarks)
- [ ] Progress is logged with current ledger position
- [ ] Task can be stopped and resumed from any ledger
- [ ] Integration test verifies backfill output triggers Lambda and produces correct database state
- [ ] Default backfill start: Soroban mainnet activation (~ledger 50,692,993), configurable via parameter

## Notes

- This is a one-time Phase 1 process. Once historical data is backfilled, the task is not needed for ongoing operation.
- The backfill rate should be tuned to avoid Lambda throttling. Start conservatively and increase based on observed Lambda concurrency and database load.
- Stellar history archives are publicly accessible and do not require authentication.
- The backfill task container image is built and pushed to ECR as part of the CI/CD pipeline (task 0076).
- Monitoring: track backfill progress via CloudWatch Logs. Compare highest backfilled ledger vs target range to estimate completion.
