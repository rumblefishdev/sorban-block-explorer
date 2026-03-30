---
id: '0041'
title: 'Galexie configuration and testnet validation'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0001', '0034']
tags: [priority-high, effort-medium, layer-indexing]
milestone: 1
links:
  - docs/architecture/technical-design-general-overview.md
history:
  - date: 2026-03-30
    status: backlog
    who: fmazur
    note: 'Task created — gap identified during milestone alignment (task 0085)'
---

# Galexie configuration and testnet validation

## Summary

Configure Galexie application (Captive Core settings, network passphrase, history archive URLs, S3 output path) and validate end-to-end on testnet before mainnet deployment. This covers the software configuration layer that sits on top of the ECS Fargate infrastructure defined in task 0034.

## Status: Backlog

**Current state:** Not started. Research task 0001 (Galexie/Captive Core setup) provides foundational knowledge. CDK infrastructure (task 0034) must exist first.

## Context

Task 0034 defines the ECS Fargate task definition, IAM roles, and S3 trigger via CDK. This task configures the actual Galexie application: Captive Core configuration file, network connection parameters, output format, and health checks. The effort breakdown (§7.1B) lists this as a separate 3-day work item: "Galexie configuration and testnet validation."

## Implementation Plan

### Step 1: Captive Core configuration

Create Captive Core configuration file (stellar-core.cfg) with appropriate settings for the target network (testnet first, then mainnet). Configure history archive URLs, network passphrase, and peer connections.

### Step 2: Galexie application configuration

Configure Galexie output settings: S3 bucket path pattern (`ledgers/{seq_start}-{seq_end}.xdr.zstd`), compression, and ledger range parameters.

### Step 3: Testnet validation

Deploy to testnet ECS Fargate and validate:

- Galexie connects to testnet peers successfully
- LedgerCloseMeta XDR files appear in S3 with correct naming
- Files arrive at expected cadence (~5-6 seconds)
- S3 PutObject events trigger Ledger Processor Lambda

### Step 4: Mainnet configuration

Switch configuration to mainnet (network passphrase, archive URLs, peer connections). Validate connectivity before enabling full ingestion.

## Acceptance Criteria

- [ ] Captive Core configuration file created for testnet and mainnet
- [ ] Galexie outputs LedgerCloseMeta XDR files to S3 with correct path pattern
- [ ] Testnet deployment produces consecutive ledger files matching network close times
- [ ] S3 PutObject events correctly trigger downstream Lambda
- [ ] Mainnet configuration prepared and connectivity validated

## Notes

- This task is the "Galexie configuration and testnet validation — 3 days" line item from the effort breakdown (§7.1B).
- Separated from 0034 (CDK infra) because infrastructure definition and application configuration are distinct concerns.
