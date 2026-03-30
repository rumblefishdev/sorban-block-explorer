---
id: '0031'
title: 'CDK: VPC, subnets, security groups, VPC endpoints'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0006']
tags: [priority-high, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# CDK: VPC, subnets, security groups, VPC endpoints

## Summary

Define the foundational AWS networking infrastructure using CDK: a VPC with public/private subnet split, security groups for inter-component access, and a VPC endpoint for S3. The network is designed for single-AZ launch in us-east-1a with a clear path to Multi-AZ expansion. All compute (Lambda, ECS Fargate) and storage (RDS) components depend on this networking layer.

## Status: Backlog

**Current state:** Not started. Research task 0006 (AWS CDK + Nx monorepo) provides foundational knowledge for the CDK setup.

## Context

The block explorer runs entirely within a dedicated AWS VPC. The network design separates public-facing delivery (CloudFront, API Gateway) from private runtime components (Lambda, ECS, RDS). This prevents direct public access to the database or ingestion workers.

The initial deployment is Single-AZ in us-east-1a to keep costs low and complexity manageable at launch. The VPC structure must support future Multi-AZ expansion without architectural changes.

Lambda functions are VPC-attached (non-default configuration) in the private subnet, which requires ENI (Elastic Network Interface) provisioning. This is intentional to allow Lambda direct access to RDS within the VPC.

### Source Code Location

- `infra/aws-cdk/lib/networking/`

## Implementation Plan

### Step 1: VPC Definition

Create a VPC with:

- CIDR block sized for growth (e.g., /16)
- Region: us-east-1
- Initial deployment: single AZ (us-east-1a)
- Structure supports adding AZs later without VPC replacement

### Step 2: Subnet Layout

Define public and private subnets:

**Public subnet (us-east-1a):**

- NAT Gateway placement (task 0040)
- Internet Gateway attachment
- Route table with 0.0.0.0/0 -> Internet Gateway

**Private subnet (us-east-1a):**

- Lambda functions (API, Ledger Processor, Event Interpreter)
- ECS Fargate tasks (Galexie live, backfill)
- RDS PostgreSQL instance
- Route table with 0.0.0.0/0 -> NAT Gateway (for outbound internet access)

### Step 3: Security Groups

Define security groups for inter-component access with least-privilege rules:

**Lambda Security Group:**

- Outbound: allow to RDS Proxy SG on PostgreSQL port (5432)
- Outbound: allow to S3 VPC endpoint (prefix list)
- Outbound: allow HTTPS (443) for AWS service API calls
- Inbound: none required (Lambda is invoked by AWS services, not by inbound connections)

**RDS Security Group:**

- Inbound: allow from Lambda SG on PostgreSQL port (5432)
- Inbound: allow from ECS SG on PostgreSQL port (5432) if ECS needs direct DB access
- Outbound: default (managed by RDS)

**ECS Security Group:**

- Outbound: allow to S3 VPC endpoint (prefix list) for Galexie S3 writes
- Outbound: allow to NAT Gateway for Stellar network peer connections and history archive access
- Outbound: allow HTTPS (443) for ECR pull and CloudWatch Logs
- Inbound: none required (ECS tasks are not externally accessible)

### Step 4: VPC Endpoint for S3

Create a Gateway VPC endpoint for S3:

- Attached to the private subnet route table
- Allows Galexie (ECS) and Ledger Processor (Lambda) to access S3 without traversing NAT Gateway
- Reduces NAT Gateway costs and improves S3 access latency

### Step 5: Multi-AZ Readiness

Structure the CDK code so that adding additional AZs requires only:

- Adding subnet definitions for new AZs
- Expanding security group rules if needed
- No changes to the VPC, endpoint, or overall architecture

Document the expansion path in CDK comments.

## Acceptance Criteria

- [ ] VPC is created with appropriate CIDR block in us-east-1
- [ ] Public subnet exists in us-east-1a with Internet Gateway route
- [ ] Private subnet exists in us-east-1a with NAT Gateway route
- [ ] Lambda Security Group allows outbound to RDS Proxy and S3 VPC endpoint
- [ ] RDS Security Group allows inbound from Lambda SG on port 5432
- [ ] ECS Security Group allows outbound to S3 VPC endpoint and NAT Gateway
- [ ] S3 Gateway VPC endpoint is configured on the private subnet route table
- [ ] Lambda functions can be VPC-attached in the private subnet (ENI configuration)
- [ ] No security group allows unrestricted inbound from 0.0.0.0/0
- [ ] CDK code is structured for Multi-AZ expansion without architectural changes
- [ ] All networking resources are tagged consistently for cost tracking
- [ ] Initial deployment in us-east-1a; VPC structured for Multi-AZ expansion without VPC replacement
- [ ] No public internet path exists to reach RDS or ECS Fargate ingestion components directly
- [ ] Multi-AZ expansion trigger documented in CDK code: expand when SLA requirement exceeds 99.9%

## Notes

- Lambda VPC attachment requires sufficient ENI capacity in the subnet. The subnet CIDR must accommodate ENI allocation for concurrent Lambda executions.
- The NAT Gateway (defined in task 0040) is placed in the public subnet. This task defines the subnet and routing; task 0040 defines the NAT Gateway and ECR resources.
- VPC Flow Logs may be added for debugging but are not required at launch.
- The VPC endpoint for S3 is a Gateway type (free) not an Interface type (has hourly cost). Gateway endpoints work for S3 and DynamoDB only.
