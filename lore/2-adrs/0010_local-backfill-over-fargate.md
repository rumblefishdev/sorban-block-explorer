---
id: '0010'
title: 'Local backfill via backfill-bench instead of AWS Fargate'
status: accepted
deciders: [fmazur, FilipDz, stkrolikiewicz]
related_tasks: ['0117', '0030']
related_adrs: ['0005']
tags: [layer-indexing, layer-infra]
links:
  - crates/backfill-bench/
history:
  - date: '2026-04-13'
    status: accepted
    who: fmazur
    note: 'Team decision — local backfill proven viable by task 0117 benchmark. Fargate approach (task 0030) superseded.'
---

# ADR 0010: Local backfill via backfill-bench instead of AWS Fargate

**Related:**

- [Task 0117: Local backfill benchmark](../1-tasks/archive/0117_FEATURE_local-backfill-benchmark.md) — implemented the tool
- [Task 0030: Fargate backfill (superseded)](../1-tasks/archive/0030_FEATURE_indexer-historical-backfill.md) — original approach, now superseded

---

## Context

The block explorer needs historical data from Soroban mainnet activation (~ledger 50,692,993)
to present — approximately 29 months of ledger data. The original plan (task 0030) was to
build an ECS Fargate task that reads from Stellar public history archives, writes XDR files
to S3, and triggers the Ledger Processor Lambda for parsing and persistence.

Task 0117 was created to benchmark indexer performance locally before committing to AWS
backfill costs. The benchmark proved that local backfill is viable and simpler.

---

## Decision

**Use local backfill via `backfill-bench` CLI tool on a workstation instead of AWS Fargate.**

The `backfill-bench` crate (`crates/backfill-bench/`) streams XDR files directly from the
Stellar public S3 bucket via HTTPS, indexes them through the same `process_ledger` pipeline
used by the Lambda, and writes directly to PostgreSQL. No AWS infrastructure (Fargate, ECS,
task definitions, IAM roles) is needed.

---

## Rationale

1. **Already built and proven.** Task 0117 delivered a working tool (PR #89 merged). It
   reuses `process_ledger` from the indexer crate — zero code duplication.

2. **Simpler infrastructure.** No Fargate task definition, ECS cluster, NAT Gateway egress
   costs, or task IAM roles to manage. Just a binary + a Postgres connection string.

3. **Lower cost.** Fargate costs would include compute time for the task + NAT Gateway
   data transfer for downloading from Stellar archives + S3 PutObject costs. Local backfill
   costs nothing beyond the workstation's electricity and internet.

4. **Same pipeline.** Both approaches use the same `process_ledger` function — data
   correctness is identical. Idempotency (ON CONFLICT DO NOTHING) and watermark logic
   work the same way.

5. **Backfill is a one-time operation.** Once historical data is loaded, the tool is not
   needed for ongoing operation. Building reusable Fargate infrastructure for a one-time
   job is over-engineering.

---

## Alternatives Considered

### Alternative 1: ECS Fargate task (original plan, task 0030)

**Description:** Dedicated Fargate task reads Stellar archives, writes XDR to S3, triggers
Lambda for processing. Supports parallel non-overlapping ranges via multiple Fargate tasks.

**Pros:**

- Runs in AWS, closer to RDS (lower latency for DB writes)
- Parallelizable via multiple Fargate tasks with non-overlapping ranges
- No dependency on a developer's workstation

**Cons:**

- Requires Fargate task definition, ECS cluster, IAM roles, VPC networking
- NAT Gateway egress costs for downloading from Stellar public S3
- S3 PutObject costs as intermediate step before Lambda processing
- More infrastructure to maintain for a one-time operation
- Not yet built (0030 was still in progress)

**Decision:** REJECTED — over-engineered for a one-time operation; local tool already works.

### Alternative 2: Direct Lambda invocation with pre-staged S3 files

**Description:** Manually download XDR files to S3, let existing Lambda event trigger handle them.

**Pros:**

- Reuses existing infrastructure entirely

**Cons:**

- Requires staging terabytes of XDR data in S3 first
- S3 storage costs during staging
- Lambda event trigger may throttle under bulk load

**Decision:** REJECTED — staging step adds unnecessary complexity and cost.

---

## Consequences

### Positive

- Zero additional AWS infrastructure to build or maintain
- Backfill can start immediately (tool is ready)
- Team can run backfill from any machine with DB access
- No ongoing cost after backfill completes

### Negative

- Backfill speed limited by workstation's network bandwidth and single-threaded processing
- Requires direct network access to the PostgreSQL database (SSH tunnel or VPN)
- If workstation goes offline during backfill, must resume manually (tool supports this
  via `--start` parameter and skip-if-exists logic)
- No built-in parallelism (sequential processing in v1) — but can be extended later if needed

---

## References

- Stellar public S3 bucket: `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`
- Pipeline audit Section 10 (parallel backfill safety): pre-backfill tasks 0118, 0119, 0130, 0134 must complete first
