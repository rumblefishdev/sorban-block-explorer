---
id: '0030'
title: 'Indexer: historical backfill Fargate task'
type: FEATURE
status: superseded
related_adr: ['0005', '0012']
related_tasks:
  [
    '0001',
    '0029',
    '0028',
    '0092',
    '0118',
    '0119',
    '0130',
    '0134',
    '0135',
    '0140',
  ]
blocked_by: ['0118', '0119', '0130', '0134']
tags: [priority-medium, effort-medium, layer-indexing, superseded-by-adr-0012]
milestone: 1
links:
  - docs/architecture/indexing-pipeline/indexing-pipeline-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: apps/indexer/ → crates/indexer/'
  - date: 2026-04-08
    status: active
    who: FilipDz
    note: 'Activated task for implementation'
  - date: '2026-04-13'
    status: active
    who: fmazur
    note: 'Added hard blockers from pipeline audit (Section 10.4): 0118, 0119, 0130, 0134 must complete before backfill runs.'
  - date: '2026-04-13'
    status: superseded
    who: fmazur
    by: ['0117']
    note: >
      Team decided to use local backfill via backfill-bench (task 0117, completed)
      instead of Fargate. backfill-bench streams from Stellar public S3 to local
      Postgres using the same process_ledger pipeline. Fargate infrastructure not
      needed. Audit pre-backfill blockers (0118, 0119, 0130, 0134) still apply to
      running backfill-bench.
  - date: '2026-04-17'
    status: superseded
    who: stkrolikiewicz
    note: >
      Flagged per task 0140 audit — implementation pattern superseded by ADR 0012
      (zero-upsert schema, insert-only history tables, S3 offload, activity
      projections, created_at partitioning). Body retained as historical record;
      do not use as reference for new work — consult ADR 0012 instead.
---

# Indexer: historical backfill Fargate task

## Summary

Implement an ECS Fargate task that reads Stellar public history archives, exports LedgerCloseMeta XDR files to the same S3 bucket used by live Galexie ingestion, and triggers the standard Ledger Processor Lambda for parsing and persistence. This enables the explorer to backfill historical data from Soroban mainnet activation onward while reusing the exact same processing pipeline as live ingestion.

## Status: Active

**Current state:** Starting implementation. Depends on the Ledger Processor (task 0029) and idempotent writes (task 0028) for downstream processing. Research task 0001 (Galexie/Captive Core setup) provides foundational knowledge.

## Pre-Backfill Blockers (audit Section 10.4)

**These tasks MUST be completed before running backfill.** Running backfill without them
causes data corruption or massive waste that is expensive to fix retroactively.

| Task                              | Why it blocks backfill                         | Consequence if skipped                                                 |
| --------------------------------- | ---------------------------------------------- | ---------------------------------------------------------------------- |
| **0118** (NFT false positives)    | Fungible transfers create spurious NFT records | Millions of false NFT records to clean up post-backfill                |
| **0119** (trustline balances)     | Dormant accounts won't self-fix after backfill | Account balances permanently incomplete for historical accounts        |
| **0130** (historical partitions)  | Only Apr-Jun 2026 partitions exist             | 29 months of data lands in DEFAULT partition, expensive to split later |
| **0134** (envelope/meta ordering) | No validation that envelopes match their metas | Silent data corruption if any ordering issue in historical data        |

**Post-backfill dependency:**
| Task | When | What |
|------|------|------|
| **0135** (holder_count) | After backfill completes | One-time full recount — inline +1/-1 must be disabled during backfill |

## Context

The block explorer needs historical chain data from Soroban mainnet activation (late 2023, approximately ledger 50,692,993) to present day. Live Galexie ingestion handles new ledgers going forward, but the historical gap must be filled by a separate backfill process.

The architecture explicitly avoids a separate parse path for backfill. Instead, the backfill task writes the same XDR file format to the same S3 bucket, which triggers the same Ledger Processor Lambda. This keeps the ingestion contract uniform and eliminates divergence between historical and live processing logic.

Live-derived state remains authoritative for the newest ledgers. Backfill data must not overwrite newer state, which is enforced by the watermark logic in task 0028.

### Source Code Location

- `crates/indexer/src/backfill/`

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

The S3 PutObject triggers the same Ledger Processor Lambda (task 0029) via the S3 event notification configured in CDK task 0032.

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

Configure as an ECS Fargate task (infrastructure in CDK task 0034):

- VPC placement: private subnet
- Outbound: NAT Gateway for Stellar archive access, S3 via VPC endpoint
- Task role: S3 PutObject on stellar-ledger-data, CloudWatch Logs
- Accept ledger range parameters (start, end) via environment variables or task overrides

### Step 6: Safety Guarantees

Ensure backfill cannot corrupt live data:

- Output goes to S3, which triggers the standard Lambda -- no direct database writes
- Watermark logic (task 0028) prevents backfill from overwriting newer live state
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
- [ ] Live-derived state is NOT overwritten by backfill (enforced by task 0028 watermarks)
- [ ] Progress is logged with current ledger position
- [ ] Task can be stopped and resumed from any ledger
- [ ] Integration test verifies backfill output triggers Lambda and produces correct database state
- [ ] Default backfill start: Soroban mainnet activation (~ledger 50,692,993), configurable via parameter

## Notes

- This is a one-time Phase 1 process. Once historical data is backfilled, the task is not needed for ongoing operation.
- The backfill rate should be tuned to avoid Lambda throttling. Start conservatively and increase based on observed Lambda concurrency and database load.
- Stellar history archives are publicly accessible and do not require authentication.
- The backfill task container image is built and pushed to ECR as part of the CI/CD pipeline (task 0039).
- Monitoring: track backfill progress via CloudWatch Logs. Compare highest backfilled ledger vs target range to estimate completion.
