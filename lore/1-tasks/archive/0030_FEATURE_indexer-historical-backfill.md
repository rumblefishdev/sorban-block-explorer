---
id: '0030'
title: 'Indexer: historical backfill Fargate task'
type: FEATURE
status: completed
related_adr: ['0005']
related_tasks: ['0001', '0029', '0028', '0092']
tags: [priority-medium, effort-medium, layer-indexing]
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
  - date: 2026-04-08
    status: completed
    who: FilipDz
    note: >
      Implemented backfill orchestrator CLI as second binary in crates/indexer.
      6 source files, 15 unit tests. Key: orchestrator delegates archive reading
      to Galexie ECS tasks, S3 gap detection for resume, semaphore-bounded concurrency.
---

# Indexer: historical backfill Fargate task

## Summary

Implement an ECS Fargate task that reads Stellar public history archives, exports LedgerCloseMeta XDR files to the same S3 bucket used by live Galexie ingestion, and triggers the standard Ledger Processor Lambda for parsing and persistence. This enables the explorer to backfill historical data from Soroban mainnet activation onward while reusing the exact same processing pipeline as live ingestion.

## Status: Done

**Current state:** Implemented. Backfill orchestrator CLI ready in `crates/indexer/src/backfill/`.

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

- [x] Fargate task reads LedgerCloseMeta from Stellar public history archives (delegated to Galexie bounded mode)
- [x] Default scope starts from Soroban mainnet activation (ledger 50,457,424)
- [x] Start and end ledger are configurable via parameters (`--start`, `--end`)
- [x] Output format matches Galexie (Galexie writes the files directly)
- [x] S3 PutObject triggers the same Ledger Processor Lambda used by live ingestion (same bucket)
- [x] Multiple parallel tasks with non-overlapping ranges work correctly (semaphore + gap detection)
- [x] No separate parse path -- all processing goes through the standard Ledger Processor
- [x] Live-derived state is NOT overwritten by backfill (enforced by task 0028 watermarks)
- [x] Progress is logged with current ledger position (per-batch structured JSON logs)
- [x] Task can be stopped and resumed from any ledger (S3 gap detection skips done ranges)
- [ ] Integration test verifies backfill output triggers Lambda and produces correct database state (deferred — requires live AWS environment with ECS/S3/RDS)
- [x] Default backfill start: Soroban mainnet activation (ledger 50,457,424), configurable via parameter

## Implementation Notes

Second `[[bin]]` target (`backfill`) in `crates/indexer/`:

| File                           | Purpose                                                         |
| ------------------------------ | --------------------------------------------------------------- |
| `src/backfill/main.rs`         | CLI entry, clap parsing, JSON tracing                           |
| `src/backfill/config.rs`       | `BackfillConfig` with all CLI args + env var fallbacks          |
| `src/backfill/range.rs`        | `LedgerRange`, `split_into_batches()`, `find_gaps()`            |
| `src/backfill/scanner.rs`      | S3 `list_objects_v2` → parse keys → covered ranges              |
| `src/backfill/runner.rs`       | ECS `RunTask` with container overrides, `DescribeTasks` polling |
| `src/backfill/orchestrator.rs` | scan → split → launch → wait → report                           |

15 unit tests in `range.rs` covering batch splitting, gap detection, merge, and realistic resume scenarios.

## Design Decisions

### From Plan

1. **Delegate archive reading to Galexie**: The orchestrator does not read Stellar history archives directly. It launches the existing Galexie Docker image (CDK backfill task definition) in bounded-range mode with START/END env var overrides. This reuses proven infrastructure.

2. **S3 gap detection for resume**: Scans S3 via `list_objects_v2` + `xdr_parser::parse_s3_key()` to find already-processed ranges. Avoids DB dependency and keeps the orchestrator lightweight.

3. **Semaphore-bounded concurrency**: `tokio::sync::Semaphore` with configurable permits (default 3) limits parallel ECS tasks.

### Emerged

4. **Second binary in indexer crate, not separate crate**: Original plan proposed `crates/backfill/`. Moved to `crates/indexer/src/backfill/` to match the task spec's source code location and keep all indexer binaries together.

5. **Task timeout**: Added `--task-timeout-secs` (default 3600) to prevent indefinite polling if an ECS task hangs. Not in original spec.

6. **ECS failure check before task ARN extraction**: RunTask can return HTTP 200 with empty tasks and non-empty failures (capacity issues). Added explicit failure check.

7. **Activation ledger 50,457,424 vs ~50,692,993**: Task spec references ~50,692,993 as rough estimate. Research task 0001 identified the precise Protocol 20 activation checkpoint as 50,457,424. Using the earlier value ensures no Soroban ledgers are missed.

## Issues Encountered

- **tracing-subscriber `json` feature**: The workspace dependency lacked the `json` feature flag. Added it to enable structured JSON logging (same format as the Lambda handler).

- **`aws-sdk-s3` not in workspace deps**: Was pinned directly as `"1"` in indexer. Moved to workspace-level dependency for consistency.

## Future Work

- End-to-end integration test in staging environment (AC #11 — requires live ECS/S3/RDS)

## Notes

- This is a one-time Phase 1 process. Once historical data is backfilled, the task is not needed for ongoing operation.
- The backfill rate should be tuned to avoid Lambda throttling. Start conservatively and increase based on observed Lambda concurrency and database load.
- Stellar history archives are publicly accessible and do not require authentication.
- The backfill task container image is built and pushed to ECR as part of the CI/CD pipeline (task 0039).
- Monitoring: track backfill progress via CloudWatch Logs. Compare highest backfilled ledger vs target range to estimate completion.
