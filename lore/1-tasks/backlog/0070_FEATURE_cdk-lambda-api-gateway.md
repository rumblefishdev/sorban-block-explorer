---
id: '0070'
title: 'CDK: Lambda functions + API Gateway'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0068', '0069']
tags: [priority-high, effort-medium, layer-infra]
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# CDK: Lambda functions + API Gateway

## Summary

Define the three Lambda functions (API, Ledger Processor, Event Interpreter) and API Gateway using CDK. All Lambdas run on Node.js ARM64/Graviton2. The API Lambda has provisioned concurrency and is triggered by API Gateway. The Ledger Processor is triggered by S3 PutObject with a DLQ. The Event Interpreter is triggered by EventBridge (configured in task 0074). API Gateway provides REST API delivery with throttling, validation, and response caching.

## Status: Backlog

**Current state:** Not started. Depends on VPC/networking (task 0068) and storage (task 0069) for subnet placement and trigger configuration.

## Context

The block explorer uses three Lambda functions for its compute layer:

1. **API Lambda (NestJS)**: Serves all public REST endpoints. Triggered by API Gateway. Provisioned concurrency to minimize cold starts for user-facing requests.
2. **Ledger Processor Lambda**: Parses XDR files and writes explorer data. Triggered by S3 PutObject events. Auto-retried on failure with a DLQ for exhausted retries.
3. **Event Interpreter Lambda**: Enriches stored events with human-readable interpretations. Triggered by EventBridge (task 0074).

All three run on ARM64 (Graviton2) for cost efficiency and are VPC-attached in the private subnet.

### Source Code Location

- `infra/aws-cdk/lib/compute/`

## Implementation Plan

### Step 1: API Lambda Definition

Define the NestJS API Lambda:

- Runtime: Node.js (latest LTS) on ARM64/Graviton2
- Handler: NestJS Lambda adapter entry point
- VPC: private subnet (task 0068)
- Security group: Lambda SG (task 0068)
- Provisioned concurrency: environment-specific (higher for production, lower for staging)
- Memory: sized for NestJS overhead + query processing
- Timeout: appropriate for API response times (e.g., 30 seconds)
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN, environment name
- IAM execution role: RDS Proxy via Secrets Manager, CloudWatch Logs, X-Ray (defined in task 0078)

### Step 2: Ledger Processor Lambda Definition

Define the Ledger Processor Lambda:

- Runtime: Node.js (latest LTS) on ARM64/Graviton2
- Trigger: S3 PutObject event (configured on stellar-ledger-data bucket in task 0069)
- VPC: private subnet
- Security group: Lambda SG
- Memory: sized for XDR parsing + database writes (higher than API Lambda)
- Timeout: sufficient for <10s target latency with margin (e.g., 60 seconds)
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN, S3 bucket name
- IAM execution role: S3 GetObject on stellar-ledger-data, RDS Proxy, CloudWatch Logs, X-Ray (task 0078)
- Auto-retry: configured by S3 event notification (default 2 retries)
- DLQ: SQS queue for exhausted retries. Failed events land here for manual investigation and replay.

### Step 3: Event Interpreter Lambda Definition

Define the Event Interpreter Lambda:

- Runtime: Node.js (latest LTS) on ARM64/Graviton2
- Trigger: EventBridge rate(5 minutes) (configured in task 0074)
- VPC: private subnet
- Security group: Lambda SG
- Memory: moderate (reads from DB, writes interpretations)
- Timeout: sufficient for batch processing (e.g., 300 seconds)
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN
- IAM execution role: RDS Proxy, CloudWatch Logs, X-Ray (task 0078)

### Step 4: API Gateway Definition

Define the REST API Gateway:

- Type: REST API (not HTTP API) for full feature support
- Integration: Lambda proxy integration with API Lambda
- Throttling: environment-specific rate and burst limits
- Request validation: enable request body and parameter validation where schemas are defined
- Response caching:
  - Long TTL for immutable data (e.g., transaction by hash, ledger by sequence)
  - Short TTL (5-15 seconds) for mutable data (e.g., recent transactions, account balances)
  - Cache keys include path + query parameters
- Stage: environment-specific (staging, production)

### Step 5: WAF Attachment

Attach the WAF WebACL (resource defined in task 0072) to the API Gateway stage. This protects the API from abuse without requiring API keys for browser traffic.

### Step 6: API Key Usage Plans

Define optional API key usage plans for non-browser consumers:

- Not required for browser traffic (the SPA does not embed API keys)
- Available for trusted automation, partner integrations, or rate-limited programmatic access
- Usage plan with throttle and quota settings

### Step 7: DLQ Configuration

Define the SQS Dead Letter Queue for the Ledger Processor:

- Receives S3 event records that exhausted Lambda retries
- Retention: 14 days (long enough for manual investigation)
- CloudWatch alarm on queue depth > 0 (indicates processing failures that need attention)
- Messages contain the original S3 event (bucket, key) for manual replay

## Acceptance Criteria

- [ ] API Lambda is defined with Node.js ARM64/Graviton2, provisioned concurrency, VPC attachment
- [ ] Ledger Processor Lambda is defined with S3 trigger, auto-retry, and SQS DLQ
- [ ] Event Interpreter Lambda is defined with appropriate timeout for batch processing
- [ ] All three Lambdas are VPC-attached in the private subnet
- [ ] API Gateway REST API is defined with Lambda proxy integration
- [ ] API Gateway throttling is configured (environment-specific)
- [ ] API Gateway response caching is configured with long TTL for immutable and short TTL for mutable data
- [ ] Cache keys include path + query parameters
- [ ] WAF WebACL from task 0072 is attached to API Gateway
- [ ] API key usage plans are defined for non-browser consumers (optional)
- [ ] SQS DLQ captures exhausted Ledger Processor retries
- [ ] IAM execution roles reference task 0078 definitions
- [ ] All environment variables are parameterized, no hard-coded values
- [ ] All three Lambda functions configured with ARM64/Graviton2 runtime
- [ ] API Lambda has provisioned concurrency configured (value from environment config task 0075)
- [ ] API Gateway enforces HTTPS/TLS for all public traffic
- [ ] Browser traffic is anonymous read-only; API keys not required for default usage
- [ ] Single Ledger Processor Lambda processes both live Galexie and historical backfill XDR files (no separate pipeline)
- [ ] No secret values embedded in Lambda deployment packages; secrets resolved at runtime via Secrets Manager
- [ ] API Lambda handlers read only from RDS PostgreSQL; no runtime dependency on Horizon, Soroswap, Aquarius, Soroban RPC, or external explorer APIs
- [ ] Failed XDR files remain in S3 after processing failure; DLQ messages contain bucket/key for manual replay
- [ ] Production Lambda database connection strings enforce TLS

## Notes

- Provisioned concurrency for the API Lambda eliminates cold starts but incurs cost even when idle. The staging environment should use a lower value.
- The DLQ is critical for operational visibility. A non-empty DLQ means ledgers are not being processed and requires investigation.
- API Gateway REST API mode (vs HTTP API) is chosen for response caching, request validation, and WAF integration. HTTP API is cheaper but lacks these features.
- Lambda ARM64/Graviton2 provides ~20% cost savings over x86_64 with comparable or better performance.
- The S3 event notification on the stellar-ledger-data bucket (task 0069) must be configured to target the Ledger Processor Lambda ARN defined here. This creates a cross-reference between tasks 0069 and 0070.
