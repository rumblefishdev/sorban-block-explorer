---
id: '0040'
title: 'CDK: IAM roles, ECR repository, NAT Gateway'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0006', '0031']
tags: [priority-high, effort-medium, layer-infra]
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
---

# CDK: IAM roles, ECR repository, NAT Gateway

## Summary

Define IAM execution roles with least-privilege permissions for all compute components, an ECR repository for the Galexie container image, and a NAT Gateway in the public subnet for ECS Fargate outbound connectivity. IAM roles cover the API Lambda, Ledger Processor (Indexer) Lambda, ECS Galexie task role, and ECS task execution role.

## Status: Backlog

**Current state:** Not started. Depends on VPC/networking (task 0031) for public subnet placement of the NAT Gateway.

## Context

Security is enforced through least-privilege IAM roles. Each compute component gets its own IAM execution role with only the permissions it needs. No shared "admin" role is used.

The NAT Gateway provides outbound internet access for ECS Fargate tasks running in the private subnet. This is required for Galexie to connect to Stellar network peers and for ECS to pull container images from ECR.

The ECR repository hosts the Galexie container image, built and pushed by the CI/CD pipeline (task 0039).

### Source Code Location

- `infra/aws-cdk/lib/security/`

## Implementation Plan

### Step 1: API Lambda Execution Role

Define IAM role for the API Lambda with least-privilege permissions:

- **RDS Proxy access**: permission to connect to RDS Proxy via IAM authentication, or permission to read database credentials from Secrets Manager
- **Secrets Manager**: `secretsmanager:GetSecretValue` on the database credentials secret ARN
- **CloudWatch Logs**: `logs:CreateLogGroup`, `logs:CreateLogStream`, `logs:PutLogEvents`
- **X-Ray**: `xray:PutTraceSegments`, `xray:PutTelemetryRecords`
- **VPC**: `ec2:CreateNetworkInterface`, `ec2:DescribeNetworkInterfaces`, `ec2:DeleteNetworkInterface` (required for VPC-attached Lambda)
- No S3 permissions (API Lambda does not access S3)

### Step 2: Ledger Processor Lambda Execution Role

Define IAM role for the Ledger Processor Lambda:

- **S3**: `s3:GetObject` on the stellar-ledger-data bucket (reads XDR files)
- **RDS Proxy access**: same as API Lambda (Secrets Manager for credentials)
- **Secrets Manager**: `secretsmanager:GetSecretValue` on database credentials secret
- **CloudWatch Logs**: same as API Lambda
- **X-Ray**: same as API Lambda
- **VPC**: same as API Lambda
- No S3 PutObject (Ledger Processor reads, does not write to S3)

### Step 3: ECS Galexie Task Role

Define IAM task role for the Galexie ECS Fargate task:

- **S3**: `s3:PutObject` on the stellar-ledger-data bucket (writes XDR files)
- **CloudWatch Logs**: `logs:CreateLogGroup`, `logs:CreateLogStream`, `logs:PutLogEvents`
- No RDS, Secrets Manager, or X-Ray permissions (Galexie writes to S3 only)

### Step 4: ECS Task Execution Role

Define the ECS task execution role (used by the ECS agent, not the application):

- **ECR**: `ecr:GetAuthorizationToken`, `ecr:BatchCheckLayerAvailability`, `ecr:GetDownloadUrlForLayer`, `ecr:BatchGetImage` (to pull container images)
- **CloudWatch Logs**: `logs:CreateLogGroup`, `logs:CreateLogStream`, `logs:PutLogEvents`
- This role is used by both the Galexie service and backfill tasks

### Step 5: ECR Repository

Define an ECR repository for the Galexie container image:

- Repository name: environment-prefixed (e.g., `prod-galexie`)
- Lifecycle policy: retain last N images, expire untagged images after 7 days
- Encryption: AES-256 (default) or KMS if required
- Image scanning: enabled for vulnerability detection
- The CI/CD pipeline (task 0039) pushes images tagged with git SHA

### Step 6: NAT Gateway

Define a NAT Gateway in the public subnet:

- Placement: public subnet in us-east-1a (from task 0031)
- Elastic IP allocation for stable outbound address
- Required for:
  - ECS Fargate outbound to Stellar network peers (Captive Core connections)
  - ECS Fargate outbound to Stellar public history archives (backfill)
  - ECS Fargate outbound to ECR for container image pull
  - Lambda outbound to AWS service endpoints not covered by VPC endpoints

**Scaling note:** Single NAT Gateway at launch. For Multi-AZ expansion, add one NAT Gateway per AZ to avoid cross-AZ traffic charges and single-AZ failure impact.

### Step 7: S3 VPC Endpoint Policy (Refinement)

Refine the S3 VPC endpoint policy (endpoint created in task 0031) to restrict access to only the project's S3 buckets:

- Allow access to stellar-ledger-data bucket
- Deny access to other S3 buckets (defense in depth)

## Acceptance Criteria

- [ ] API Lambda role: RDS Proxy (via Secrets Manager), CloudWatch Logs, X-Ray, VPC networking -- no S3
- [ ] Ledger Processor role: S3 GetObject on stellar-ledger-data, RDS Proxy, CloudWatch Logs, X-Ray, VPC networking -- no S3 PutObject
- [ ] ECS Galexie task role: S3 PutObject on stellar-ledger-data, CloudWatch Logs -- no RDS
- [ ] ECS task execution role: ECR pull, CloudWatch Logs
- [ ] All roles follow least-privilege principle (no wildcard actions or resources)
- [ ] ECR repository is defined with lifecycle policy and image scanning
- [ ] NAT Gateway is placed in the public subnet with Elastic IP
- [ ] Private subnet route table routes 0.0.0.0/0 through NAT Gateway
- [ ] NAT Gateway documentation notes Single NAT at launch, expandable per-AZ
- [ ] S3 VPC endpoint policy restricts access to project buckets only
- [ ] GitHub Actions OIDC identity provider resource defined in CDK
- [ ] Separate IAM deploy roles for staging and production with repository and branch scope conditions
- [ ] Secrets Manager IAM permissions scoped to specific secret ARNs per workload (not wildcard)

## Notes

- IAM roles should use resource-level ARN restrictions wherever possible. Avoid `Resource: "*"` except for actions that require it (e.g., `ec2:DescribeNetworkInterfaces`).
- The NAT Gateway incurs hourly cost plus data transfer charges. S3 traffic should route through the VPC endpoint (free) rather than NAT Gateway. Only Stellar network peer traffic and ECR pulls should traverse NAT.
- ECR image lifecycle policy prevents unbounded image storage growth. Keep last 10-20 images for rollback capability.
- The ECS task execution role is distinct from the task role. The execution role is used by the ECS agent for infrastructure operations (image pull, log creation). The task role is used by the application code for business operations (S3 writes).
- For production, consider adding IAM access analyzer to detect overly permissive policies.
