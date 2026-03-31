# AWS CDK Infrastructure

CDK stacks for the Soroban Block Explorer. Defines all AWS resources: networking, storage, compute, delivery, and monitoring.

## Stack architecture

```
NetworkStack (VPC, subnets, security groups, VPC endpoints)
    |
StorageStack (RDS, RDS Proxy, S3, Secrets Manager)
    |
    +-- ApiStack (API Lambda, API Gateway)
    +-- IndexerStack (Rust Ledger Processor Lambda, SQS DLQ)
    +-- IngestionStack (ECS Fargate Galexie, ECR)
    +-- FrontendStack (CloudFront, WAF, Route 53)
    |
MonitoringStack (CloudWatch, alarms, X-Ray)
```

Currently implemented: **NetworkStack**.

## Prerequisites

- AWS CLI with a configured profile
- Node.js 22+
- `export AWS_PROFILE=soroban-explorer`

## Commands

From `infra/aws-cdk/`:

```bash
# First-time setup (once per AWS account + region)
make bootstrap

# Staging
make diff-staging              # Preview changes
make deploy-staging            # Deploy all stacks
make deploy-staging-network    # Deploy single stack

# Production
make diff-production
make deploy-production
make deploy-production-network
```

Or from the repository root:

```bash
npm run infra:diff:staging
npm run infra:deploy:staging
```

## Environments

Each environment has its own JSON config and CDK entry point:

| Environment | Config                 | Entry point             | VPC CIDR    | NAT             |
| ----------- | ---------------------- | ----------------------- | ----------- | --------------- |
| staging     | `envs/staging.json`    | `src/bin/staging.ts`    | 10.0.0.0/16 | Managed gateway |
| production  | `envs/production.json` | `src/bin/production.ts` | 10.1.0.0/16 | Managed gateway |

## Project structure

```
envs/
  staging.json               # Staging environment config
  production.json            # Production environment config
src/
  bin/
    staging.ts               # CDK app entry point — staging
    production.ts            # CDK app entry point — production
  lib/
    types.ts               # EnvironmentConfig interface
    app.ts                 # Shared stack wiring (createApp)
    stacks/
      network-stack.ts     # VPC, subnets, SGs, S3 VPC endpoint
Makefile                     # Deploy/synth/diff targets per environment
```

## NetworkStack resources

- VPC with /16 CIDR in us-east-1
- Public subnet (/20) with Internet Gateway
- Private subnet (/20) with NAT Gateway
- Lambda security group (outbound: RDS 5432, HTTPS 443)
- RDS security group (inbound: Lambda + ECS on 5432)
- ECS security group (outbound: HTTPS 443, RDS 5432, Stellar peers 11625)
- S3 Gateway VPC endpoint on private subnet route table

Single-AZ deployment in us-east-1a. Multi-AZ expansion requires only config changes.
