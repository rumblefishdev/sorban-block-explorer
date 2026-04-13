---
id: '0036'
title: 'CDK: CloudWatch dashboards and alarms'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0006', '0108']
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
    note: 'Updated: removed Event Interpreter references. Architecture simplified to 2 Lambdas (API + Indexer).'
  - date: 2026-04-09
    status: active
    who: FilipDz
    note: 'Activated for implementation'
  - date: 2026-04-10
    status: completed
    who: FilipDz
    note: >
      CDK-only implementation. New CloudWatchStack: 1 SNS topic + AWS Chatbot
      SlackChannelConfiguration (SNS → Chatbot → Slack channel),
      5 alarms (Galexie lag, Processor error rate, RDS CPU, RDS storage, API 5xx),
      CloudWatch dashboard with 11 widgets across Ingestion/API/Resources sections.
      Galexie lag uses Processor Lambda Invocations=0 as S3 freshness proxy.
      RDS storage threshold derived from rdsStorageThresholdPct × dbAllocatedStorage.
      6 new EnvironmentConfig fields (slackWorkspaceId, slackChannelId, thresholds)
      + validateConfig checks. Indexed-vs-tip and custom LastProcessedLedgerSequence
      metric deferred (requires Rust changes). Prerequisite: authorize Slack workspace
      in AWS Chatbot console once per account before cdk deploy.
      6 files changed: cloudwatch-stack.ts (new), app.ts, types.ts, index.ts,
      envs/staging.json, envs/production.json.
---

# CDK: CloudWatch dashboards and alarms

## Summary

Define CloudWatch dashboards for operational visibility and alarms for critical system health indicators. Dashboards cover Galexie S3 freshness, Ledger Processor performance, API latency percentiles, RDS resource utilization, Lambda concurrency and cold starts, and indexed-vs-tip gap. Alarms trigger SNS notifications with environment-specific severity (production paging, staging email/Slack only).

## Status: Backlog

**Current state:** Not started. Alarm thresholds are environment-configurable via task 0038.

## Context

Observability is a core infrastructure concern. The block explorer needs operational dashboards for day-to-day monitoring and alarms for automated incident detection. The monitoring baseline is defined in the infrastructure overview and covers the full pipeline from Galexie ingestion through API delivery.

### Source Code Location

- `infra/aws-cdk/lib/observability/dashboards.ts`

## Implementation Plan

### Step 1: Dashboards

Define CloudWatch dashboards with the following widgets:

**Ingestion Health:**

- Galexie S3 freshness: timestamp of latest S3 object vs current time
- Indexed ledger vs network tip: gap between highest indexed ledger and network tip
- Ledger Processor duration: p50, p95, p99 execution time
- Ledger Processor error count and rate

**API Performance:**

- API Lambda latency: p50, p95, p99
- API Gateway request count and 4xx/5xx rates
- API Gateway cache hit rate

**Resource Utilization:**

- RDS CPU utilization
- RDS connection count (active vs max)
- RDS free storage space
- Lambda concurrency utilization (concurrent executions vs limit)
- Lambda cold start rate per function
- Lambda duration per function (API, Ledger Processor)

### Step 2: Alarm Definitions

Define alarms with evaluation periods:

**Galexie ingestion lag (hang detection):**

- Condition: S3 timestamps more than 60 seconds behind current time (compare latest object creation time vs wall clock)
- Evaluation: 3 consecutive datapoints of 1-minute period, all breaching
- Severity: high (ingestion is stalled)
- Note: This is the hang detection layer. Task 0108 adds a process-level health check (`pgrep`) that catches crashes but not silent hangs where Galexie is alive but not producing data. This alarm covers that gap.

**Ledger Processor error rate:**

- Condition: >1% of Lambda invocations result in errors
- Evaluation: 5-minute window
- Severity: high (ledgers are failing to process)

**RDS CPU:**

- Condition: >70% sustained
- Evaluation: 5 datapoints of 1-minute period, all breaching (5 minutes sustained)
- Severity: medium (may need scaling or query optimization)

**RDS free storage:**

- Condition: <20% remaining
- Evaluation: 1 datapoint (immediate alert)
- Severity: high (risk of full disk)

**API Gateway 5xx rate:**

- Condition: >0.5% of requests result in 5xx
- Evaluation: 5-minute window
- Severity: high (user-facing errors)

### Step 3: SNS Notification Targets

Define SNS topics for alarm notifications:

**Production:**

- SNS topic for paging/PagerDuty integration
- High-severity alarms trigger paging
- Medium-severity alarms trigger email notification

**Staging:**

- SNS topic for email/Slack only (non-paging)
- Potentially relaxed thresholds (higher error rates tolerated, longer evaluation windows)
- Purpose: catch regressions, not page on-call

### Step 4: Environment-Configurable Thresholds

All alarm thresholds are parameterized and configured via the environment config module (task 0038):

- Production: strict thresholds as defined above
- Staging: relaxed thresholds, non-paging notifications
- Threshold values are not hard-coded in the alarm definitions

## Acceptance Criteria

- [x] CloudWatch dashboard includes Galexie S3 freshness widget (via Processor Invocations proxy)
- [x] Dashboard includes Ledger Processor duration and error rate widgets
- [x] Dashboard includes API latency p50/p95/p99 widgets
- [x] Dashboard includes RDS CPU and connection count widgets
- [ ] Dashboard includes indexed vs network tip gap widget — **deferred** (requires Rust custom metric)
- [x] Dashboard includes Lambda concurrency utilization and cold start rate widgets
- [x] Dashboard includes Lambda duration per function widgets
- [x] Galexie ingestion lag alarm fires when Processor Invocations = 0 for N minutes
- [x] Ledger Processor error rate alarm fires above threshold of invocations
- [x] RDS CPU alarm fires above threshold sustained for 5 minutes
- [x] RDS free storage alarm fires below threshold% remaining
- [x] API Gateway 5xx alarm fires above threshold% of requests
- [x] Production: alarms trigger SNS topic (paging email — fill in alarmEmail before deploy)
- [x] Staging: alarms trigger SNS topic (ops email, non-paging)
- [x] All thresholds are environment-configurable via EnvironmentConfig
- [x] Alarm thresholds match architecture baseline: Galexie lag, Processor error, RDS CPU sustained 5min, RDS storage, API 5xx
- [x] Staging defines the same five alarm categories as production, differing only in thresholds
- [x] Production alarm thresholds documented in CDK config (comments in EnvironmentConfig interface)
- [x] Staging alarm thresholds tuned for regression detection (relaxed vs production)

## Notes

- The "indexed vs network tip" metric requires a custom metric. The Ledger Processor can publish the highest processed ledger sequence to CloudWatch, and a separate check can compare it against the Stellar network tip (e.g., via Horizon or Galexie health).
- Dashboard widgets should use STAT periods aligned with alarm evaluation periods for consistency.
- Additional alarms can be added incrementally (e.g., DLQ depth from task 0033). The initial set covers the documented baseline.
- CloudWatch Logs Insights queries may complement dashboards for ad-hoc investigation but are not defined as CDK resources.
