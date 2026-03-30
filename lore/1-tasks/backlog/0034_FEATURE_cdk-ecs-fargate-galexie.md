---
id: '0034'
title: 'CDK: ECS Fargate for Galexie live + backfill'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0006', '0031', '0001']
tags: [priority-medium, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# CDK: ECS Fargate for Galexie live + backfill

## Summary

Define ECS Fargate infrastructure for two workloads: (1) a continuous Galexie service for live ledger export, and (2) a one-time backfill Fargate task for historical data. Both run in the private subnet, output to the stellar-ledger-data S3 bucket, and are configured with appropriate health checks, restart policies, and task roles.

## Status: Backlog

**Current state:** Not started. Depends on VPC/networking (task 0031) for subnet placement. Research task 0001 (Galexie/Captive Core setup) provides foundational knowledge.

## Context

Galexie is the live data export process that connects to Stellar network peers via Captive Core and produces one LedgerCloseMeta XDR file per ledger close (~5-6 seconds). It runs continuously as an ECS Fargate service.

The backfill workload reads from Stellar public history archives and produces the same XDR file format. It runs as a one-time (or few-times) ECS Fargate task, not a continuous service.

Both workloads write to `stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd` in S3. The S3 PutObject event then triggers the Ledger Processor Lambda (same processing path for both live and backfill).

### Source Code Location

- `infra/aws-cdk/lib/ingestion/`

## Implementation Plan

### Step 1: ECS Cluster Definition

Define an ECS cluster for ingestion workloads. Both the Galexie service and backfill tasks run in this cluster.

### Step 2: Galexie Continuous Service

Define an ECS Fargate service for live Galexie:

- Task definition: Galexie container image (from ECR, task 0040)
- CPU/memory: sized for Captive Core + Galexie export overhead
- VPC placement: private subnet (task 0031)
- Security group: ECS SG (task 0031)
- Desired count: 1 (single instance)
- Restart policy: automatic restart on failure for continuous recovery
- Network mode: awsvpc

**Captive Core configuration:**

- Network passphrase: environment-driven
  - Production: Stellar mainnet passphrase
  - Staging: Stellar testnet passphrase
- Passphrase passed via environment variable or mounted config

**S3 output:**

- Target bucket: stellar-ledger-data
- Key pattern: `ledgers/{seq_start}-{seq_end}.xdr.zstd`
- Cadence: ~5-6 seconds per ledger close

**Checkpoint-aware restart:**

- Galexie resumes from the last exported ledger on restart
- No data loss on service restart or container replacement

**Health check:**

- Verify S3 object production within expected cadence
- If no new S3 object appears within a configurable window (e.g., 2 minutes), flag as unhealthy
- ECS replaces unhealthy tasks automatically

### Step 3: Backfill Fargate Task Definition

Define an ECS Fargate task definition for historical backfill:

- Separate task definition from the Galexie service
- Container image: backfill application (from ECR)
- CPU/memory: sized for archive reading and S3 writing
- VPC placement: private subnet
- Security group: ECS SG

**Configurable parameters:**

- Start ledger sequence (via environment variable or task override)
- End ledger sequence (via environment variable or task override)
- Default: Soroban mainnet activation (~ledger 50,692,993) to current tip

**Parallel execution:**

- Multiple tasks can run simultaneously with non-overlapping ledger ranges
- Each task is independent; no shared state between parallel tasks

**One-time execution model:**

- Run as ECS RunTask, not as a service
- Terminates when the specified range is complete
- Can be re-run for any range if needed

### Step 4: VPC Placement and Networking

Both workloads in the private subnet:

- Outbound to Stellar network peers: via NAT Gateway (task 0040)
- Outbound to Stellar history archives: via NAT Gateway
- Outbound to S3: via VPC endpoint (task 0031) -- avoids NAT Gateway costs for S3 traffic
- Outbound to ECR: via NAT Gateway for image pull
- Outbound to CloudWatch Logs: via NAT Gateway

### Step 5: ECS Task Roles

Define IAM roles for ECS tasks (detailed permissions in task 0040):

**Galexie task role:**

- S3 PutObject on stellar-ledger-data bucket
- CloudWatch Logs write

**Task execution role:**

- ECR image pull
- CloudWatch Logs write

## Acceptance Criteria

- [ ] ECS cluster is defined for ingestion workloads
- [ ] Galexie continuous service runs as ECS Fargate with desired count 1
- [ ] Galexie restart policy ensures automatic recovery on failure
- [ ] Captive Core network passphrase is environment-driven (mainnet for prod, testnet for staging)
- [ ] S3 output matches pattern: `stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd`
- [ ] Galexie is checkpoint-aware and resumes from last exported ledger
- [ ] Health check verifies S3 object production within expected cadence
- [ ] Backfill task definition accepts configurable start/end ledger range
- [ ] Multiple parallel backfill tasks with non-overlapping ranges are supported
- [ ] Both workloads run in the private subnet with correct security group
- [ ] S3 access routes through VPC endpoint, not NAT Gateway
- [ ] Stellar network access routes through NAT Gateway
- [ ] Task roles grant appropriate S3 and CloudWatch permissions
- [ ] No secret values baked into Galexie container image; all secrets resolved at runtime via Secrets Manager or environment variables referencing secret ARNs
- [ ] ECS tasks connect only to Stellar network peers and Stellar public history archives as external data sources; no other external API dependency

## Notes

- The Galexie container image must be built and pushed to ECR as part of CI/CD (task 0039). The ECS task definition references the ECR repository (task 0040).
- Captive Core within Galexie requires outbound connectivity to Stellar network peers on various ports. The NAT Gateway in the public subnet (task 0040) provides this.
- The health check based on S3 object production cadence is an application-level health signal. If ledger closes slow down on the Stellar network itself, the health check window should account for this.
- Backfill parallelism should be limited by database write throughput. Start with 2-3 parallel tasks and increase based on observed Ledger Processor Lambda and RDS performance.
