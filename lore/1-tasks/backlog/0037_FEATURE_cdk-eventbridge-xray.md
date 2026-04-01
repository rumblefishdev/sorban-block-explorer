---
id: '0037'
title: 'CDK: X-Ray tracing'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0006']
tags: [priority-medium, effort-small, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-04-01
    status: backlog
    who: fmazur
    note: 'Updated: removed Event Interpreter references. Architecture simplified to 2 Lambdas (API + Indexer). EventBridge rule for interpreter deferred.'
---

# CDK: X-Ray tracing

## Summary

Define X-Ray distributed tracing configuration using CDK. X-Ray active tracing is enabled on API Gateway, both Lambda functions (API + Indexer), and RDS Proxy (if supported). Sampling rules are environment-specific.

> **Note:** The original EventBridge scheduling rule for the Event Interpreter Lambda has been removed. The architecture has been simplified to 2 Lambdas: API Lambda and Ledger Processor/Indexer Lambda. The pipeline is: Galexie -> S3 -> Indexer Lambda -> PostgreSQL <- API Lambda <- Frontend. If EventBridge scheduling is needed in the future, it can be added as a separate task.

## Status: Backlog

**Current state:** Not started. No blocking dependencies, but coordinates with Lambda definitions in task 0033 (which defines 2 Lambdas: API + Indexer).

## Context

X-Ray provides distributed tracing across the API request path (API Gateway -> Lambda -> RDS Proxy -> RDS) and the ingestion path (S3 -> Lambda -> RDS Proxy -> RDS). This is essential for debugging latency issues and understanding request flow.

### Source Code Location

- `infra/aws-cdk/lib/observability/tracing.ts`

## Implementation Plan

### Step 1: X-Ray Active Tracing - API Gateway

Enable X-Ray tracing on the API Gateway stage:

- Active tracing captures request/response metadata and latency
- Integrated with the API Lambda traces for end-to-end visibility

### Step 2: X-Ray Active Tracing - Lambda Functions

Enable X-Ray active tracing on both Lambda functions:

- API Lambda: traces API request processing, DB queries
- Ledger Processor/Indexer Lambda: traces XDR parsing, DB writes

Each Lambda automatically creates X-Ray segments and subsegments for AWS SDK calls (S3, RDS, Secrets Manager).

### Step 3: X-Ray Active Tracing - RDS Proxy

Enable X-Ray tracing on RDS Proxy if supported by the current CDK/RDS Proxy version. This adds visibility into connection pooling and query forwarding latency.

If RDS Proxy X-Ray integration is not available via CDK, document the limitation and rely on Lambda-side database call tracing.

### Step 4: Sampling Rules

Define X-Ray sampling rules that are environment-specific:

**Production:**

- Lower sampling rate (e.g., 5% of requests) to reduce cost and noise
- Fixed rate for critical paths (e.g., always trace errors)
- Reservoir: small fixed number of traces per second guaranteed

**Staging:**

- Higher sampling rate (e.g., 50-100%) for debugging and development
- Useful for tracing through the full pipeline during testing

Sampling rules are configured as X-Ray sampling rule resources in CDK.

## Acceptance Criteria

- [ ] X-Ray active tracing enabled on API Gateway stage
- [ ] X-Ray active tracing enabled on both Lambda functions (API + Indexer)
- [ ] X-Ray tracing on RDS Proxy if supported (documented limitation if not)
- [ ] Production sampling rule: lower rate for cost efficiency
- [ ] Staging sampling rule: higher rate for debugging

## Notes

- X-Ray traces from the Ledger Processor/Indexer are particularly valuable for diagnosing slow ledger processing. They can reveal whether time is spent in XDR parsing, DB writes, or S3 download.
- X-Ray sampling rules prevent excessive trace data in production while maintaining full visibility in staging.
