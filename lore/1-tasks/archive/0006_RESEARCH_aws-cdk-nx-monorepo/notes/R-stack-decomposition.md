---
title: 'CDK stack decomposition and dependency ordering'
type: research
status: developing
spawned_from: null
spawns: []
tags: [cdk, stacks, architecture]
links:
  - https://docs.aws.amazon.com/cdk/v2/guide/stacks.html
history:
  - date: 2026-03-26
    status: developing
    who: stkrolikiewicz
    note: 'Initial decomposition based on architecture docs and deployment ordering'
---

# CDK stack decomposition and dependency ordering

## Approach: Layer-Based Stacks

One stack per infrastructure layer, not per service. This matches the deployment ordering and minimizes cross-stack references while keeping each stack focused.

## Recommended Stack Structure

```
infra/aws-cdk/
├── bin/
│   └── app.ts                    # CDK app entry — instantiates all stacks
├── lib/
│   ├── stacks/
│   │   ├── network-stack.ts      # VPC, subnets, SGs, VPC endpoints, NAT Gateway
│   │   ├── storage-stack.ts      # RDS, RDS Proxy, S3 buckets, Secrets Manager
│   │   ├── compute-stack.ts      # Lambdas (Rust + Node.js), ECS Fargate (Galexie), ECR, SQS DLQ
│   │   ├── delivery-stack.ts     # API Gateway, CloudFront, WAF, Route 53, ACM certificates
│   │   ├── monitoring-stack.ts   # CloudWatch dashboards, alarms, X-Ray, EventBridge
│   │   └── ci-stack.ts           # OIDC provider, deploy IAM roles (per ADR-0001)
│   ├── config/
│   │   ├── index.ts              # Config loader
│   │   ├── staging.ts            # Staging-specific values
│   │   └── production.ts         # Production-specific values
│   └── constructs/
│       ├── rust-lambda.ts        # Rust Lambda construct wrapper
│       ├── nestjs-lambda.ts      # NestJS Lambda construct wrapper
│       └── galexie-service.ts    # ECS Fargate Galexie construct
├── cdk.json
├── package.json
└── tsconfig.json
```

## Deployment Order (Stack Dependencies)

```
CiStack (OIDC, deploy roles)
    ↓
NetworkStack (VPC, subnets, SGs, endpoints)
    ↓
StorageStack (RDS, RDS Proxy, S3, Secrets Manager)
    ↓
ComputeStack (Lambdas, ECS Fargate)
    ↓
DeliveryStack (API Gateway, CloudFront, WAF, Route 53)
    ↓
MonitoringStack (CloudWatch, alarms, X-Ray, EventBridge)
```

### Why this order:

1. **CiStack first** — OIDC provider and deploy roles must exist before any other stack can be deployed via GitHub Actions
2. **NetworkStack** — VPC is referenced by everything that needs private networking
3. **StorageStack** — RDS, S3, Secrets Manager are referenced by compute (Lambda needs DB, S3 trigger)
4. **ComputeStack** — Lambda functions and ECS tasks reference VPC, RDS, S3
5. **DeliveryStack** — API Gateway references API Lambda, CloudFront references S3 buckets
6. **MonitoringStack** — alarms reference Lambda, RDS, API Gateway metrics

### Cross-Stack References

CDK handles cross-stack references via CloudFormation exports. Key references:

| From            | To            | What                                                       |
| --------------- | ------------- | ---------------------------------------------------------- |
| ComputeStack    | —             | ECR repository (owned by ComputeStack, no cross-stack ref) |
| StorageStack    | NetworkStack  | VPC, private subnets, security groups                      |
| ComputeStack    | NetworkStack  | VPC, subnets, SGs                                          |
| ComputeStack    | StorageStack  | RDS Proxy endpoint, S3 bucket ARNs, secret ARNs            |
| DeliveryStack   | ComputeStack  | API Lambda function ARN                                    |
| DeliveryStack   | StorageStack  | api-docs S3 bucket, frontend S3 bucket                     |
| MonitoringStack | ComputeStack  | Lambda function names                                      |
| MonitoringStack | StorageStack  | RDS instance ID                                            |
| MonitoringStack | DeliveryStack | API Gateway ID                                             |

## Stack vs Construct Decision

**Stacks** = deployment units (CloudFormation stacks). Independent deploy, rollback, IAM boundaries.

**Constructs** = reusable components within a stack. Shared patterns (e.g., `RustLambda` construct used in ComputeStack).

Rule: if two resources MUST be deployed together → same stack. If they CAN be deployed independently → separate stacks.

## Alternative Considered: Single Stack

A single "ExplorerStack" with all resources.

**Pros:** No cross-stack references, simpler initial setup.
**Cons:** All-or-nothing deployment, 500-resource CloudFormation limit risk, slow deploys, can't independently update monitoring without redeploying compute.

**Rejected** for a system with ~20+ AWS resources that will grow.

## Alternative Considered: Per-Service Stacks

Separate stacks for each service (LambdaProcessorStack, GalexieStack, RdsStack, etc.).

**Pros:** Maximum isolation.
**Cons:** Too many stacks (~10+), complex cross-stack reference web, slow parallel deployment, harder to reason about.

**Rejected** as over-granular for a team of 2.

## Multi-AZ Expansion

The stack structure supports Multi-AZ expansion without replacement:

- `NetworkStack` parameters control AZ count
- `StorageStack` RDS `multiAz` flag is a config toggle
- No stack restructuring needed — just config change in `staging.ts` / `production.ts`
