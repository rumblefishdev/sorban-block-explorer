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

From the **repository root**:

```bash
# First-time setup (once per AWS account + region)
npm run infra:bootstrap

# Preview what will change
npm run infra:diff -- --context env=staging

# Deploy
npm run infra:deploy -- --context env=staging

# Generate CloudFormation template without deploying
npm run infra:synth -- --context env=staging
```

Or via Nx directly:

```bash
npx nx deploy @rumblefish/soroban-block-explorer-aws-cdk -- --context env=staging
```

## Environments

| Environment | VPC CIDR    | Config file                    |
| ----------- | ----------- | ------------------------------ |
| staging     | 10.0.0.0/16 | `src/lib/config/staging.ts`    |
| production  | 10.1.0.0/16 | `src/lib/config/production.ts` |

Environment is selected via `--context env=staging` or `--context env=production`.

## Project structure

```
src/
  bin/
    app.ts                 # CDK app entry point
  lib/
    config/
      types.ts             # EnvironmentConfig interface
      staging.ts           # Staging values
      production.ts        # Production values
      index.ts             # Config resolver
    stacks/
      network-stack.ts     # VPC, subnets, SGs, S3 VPC endpoint
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
