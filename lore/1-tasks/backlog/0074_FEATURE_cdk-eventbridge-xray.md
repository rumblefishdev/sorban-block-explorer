---
id: '0074'
title: 'CDK: EventBridge rules and X-Ray tracing'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-medium, effort-small, layer-infra]
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# CDK: EventBridge rules and X-Ray tracing

## Summary

Define EventBridge scheduling rules and X-Ray distributed tracing configuration using CDK. EventBridge triggers the Event Interpreter Lambda every 5 minutes with retry policy and DLQ. X-Ray active tracing is enabled on API Gateway, all three Lambda functions, and RDS Proxy (if supported). Sampling rules are environment-specific.

## Status: Backlog

**Current state:** Not started. No blocking dependencies, but coordinates with Lambda definitions in task 0070.

## Context

EventBridge provides the scheduling mechanism for the Event Interpreter Lambda, which runs as a periodic enrichment job independent of the primary ingestion pipeline.

X-Ray provides distributed tracing across the API request path (API Gateway -> Lambda -> RDS Proxy -> RDS) and the ingestion path (S3 -> Lambda -> RDS Proxy -> RDS). This is essential for debugging latency issues and understanding request flow.

### Source Code Location

- `infra/aws-cdk/lib/observability/scheduling.ts`

## Implementation Plan

### Step 1: EventBridge Rule for Event Interpreter

Define an EventBridge rule:

- Schedule expression: `rate(5 minutes)`
- Target: Event Interpreter Lambda (from task 0070)
- IAM permission: EventBridge to invoke the Lambda function

**Retry policy:**

- Maximum retries: 2
- On exhausted retries: send to DLQ (SQS)

**DLQ for EventBridge:**

- SQS queue to capture failed EventBridge-to-Lambda invocations
- Separate from the Ledger Processor DLQ (task 0070)
- Retention: 14 days
- Alarm on queue depth > 0 (coordinates with task 0073)

### Step 2: X-Ray Active Tracing - API Gateway

Enable X-Ray tracing on the API Gateway stage:

- Active tracing captures request/response metadata and latency
- Integrated with the API Lambda traces for end-to-end visibility

### Step 3: X-Ray Active Tracing - Lambda Functions

Enable X-Ray active tracing on all three Lambda functions:

- API Lambda: traces API request processing, DB queries
- Ledger Processor Lambda: traces XDR parsing, DB writes
- Event Interpreter Lambda: traces event queries, interpretation writes

Each Lambda automatically creates X-Ray segments and subsegments for AWS SDK calls (S3, RDS, Secrets Manager).

### Step 4: X-Ray Active Tracing - RDS Proxy

Enable X-Ray tracing on RDS Proxy if supported by the current CDK/RDS Proxy version. This adds visibility into connection pooling and query forwarding latency.

If RDS Proxy X-Ray integration is not available via CDK, document the limitation and rely on Lambda-side database call tracing.

### Step 5: Sampling Rules

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

- [ ] EventBridge rule triggers Event Interpreter Lambda at rate(5 minutes)
- [ ] EventBridge retry policy: max 2 retries with SQS DLQ on exhaustion
- [ ] IAM permission allows EventBridge to invoke the Event Interpreter Lambda
- [ ] X-Ray active tracing enabled on API Gateway stage
- [ ] X-Ray active tracing enabled on all three Lambda functions
- [ ] X-Ray tracing on RDS Proxy if supported (documented limitation if not)
- [ ] Production sampling rule: lower rate for cost efficiency
- [ ] Staging sampling rule: higher rate for debugging
- [ ] DLQ alarm integrates with monitoring (task 0073)

## Notes

- The 5-minute EventBridge cadence means the Event Interpreter runs approximately 288 times per day. Each invocation should be fast (seconds, not minutes) under normal conditions.
- X-Ray traces from the Ledger Processor are particularly valuable for diagnosing slow ledger processing. They can reveal whether time is spent in XDR parsing, DB writes, or S3 download.
- X-Ray sampling rules prevent excessive trace data in production while maintaining full visibility in staging.
- EventBridge rules are region-specific. If the stack moves regions, the rule moves with it (defined in CDK, not manually).
