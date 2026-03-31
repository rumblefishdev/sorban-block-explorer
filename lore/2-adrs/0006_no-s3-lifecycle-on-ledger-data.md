---
id: '0006'
title: 'No S3 lifecycle rules on ledger data bucket'
status: accepted
deciders: [fmazur]
related_tasks: ['0032']
related_adrs: []
tags: [infrastructure, storage, s3]
links: []
history:
  - date: 2026-03-31
    status: accepted
    who: fmazur
    note: 'Decision made during task 0032 implementation'
---

# ADR 0006: No S3 lifecycle rules on ledger data bucket

## Context

The `stellar-ledger-data` S3 bucket stores LedgerCloseMeta XDR files from Galexie (live ingestion) and backfill. After Lambda Processor parses these files, the extracted data lives in RDS PostgreSQL.

The original task spec proposed automatic deletion after 7 days (staging) / 30 days (production) to reduce storage costs.

## Decision

No lifecycle rules on the ledger data bucket. Files are retained indefinitely on both staging and production.

## Rationale

- The project is in early development — we don't yet know what replay or debugging scenarios will arise
- S3 storage costs for XDR files are low relative to other infrastructure costs (RDS, NAT Gateway)
- Premature deletion removes the ability to reprocess ledgers if the Lambda Processor logic changes
- Lifecycle rules can be added later with a single config change when storage costs become a concern

## Consequences

- S3 storage costs will grow over time proportionally to ingested ledger count
- Full reprocessing from raw XDR files remains possible at any point
- Monitor S3 costs in CloudWatch; revisit this decision if costs exceed ~$20/mo
