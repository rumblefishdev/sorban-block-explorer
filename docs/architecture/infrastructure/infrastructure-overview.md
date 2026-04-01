# Stellar Block Explorer - Infrastructure Overview

> This document expands the infrastructure portion of
> [`technical-design-general-overview.md`](../technical-design-general-overview.md).
> It preserves the same hosting, deployment, and operational assumptions, but specifies the
> infrastructure model in more detail so it can later serve as input for implementation task
> planning.

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Infrastructure Principles](#2-infrastructure-principles)
3. [Target System Topology](#3-target-system-topology)
4. [Deployment Model](#4-deployment-model)
5. [Managed Components](#5-managed-components)
6. [Networking and Security Boundary](#6-networking-and-security-boundary)
7. [Environments and Scalability](#7-environments-and-scalability)
8. [Observability and Operations](#8-observability-and-operations)
9. [Delivery Model and Workspace Boundary](#9-delivery-model-and-workspace-boundary)

---

## 1. Purpose and Scope

Infrastructure is the AWS-hosted runtime foundation of the block explorer. Its role is to
run ingestion, storage, API delivery, static frontend hosting, and operational monitoring in
one deployable system that can be operated without depending on third-party explorer
services.

This document covers the target infrastructure design only. It does not redefine frontend
behavior, backend API contracts, indexing logic, or database schema beyond the parts needed
to explain how the infrastructure is deployed and operated.

This document describes the intended production infrastructure model. It is not a
reflection of the current implementation state in the repository, which is still skeletal.

If any statement in this file conflicts with
[`technical-design-general-overview.md`](../technical-design-general-overview.md), the main
overview document takes precedence. This file is an infrastructure-focused refinement of
that source, not an independent redesign.

## 2. Infrastructure Principles

The source design implies a small set of infrastructure principles that should remain
stable unless the main document changes first.

### 2.1 Full-Stack Ownership

The block explorer runs on infrastructure owned by the project team in a dedicated AWS
sub-account. Core functionality does not depend on Horizon, Soroswap, Aquarius, Soroban
RPC, or any external explorer API.

The infrastructure is expected to host:

- canonical ledger ingestion
- explorer database storage
- public REST API delivery
- public frontend delivery
- operational visibility and alarms

### 2.2 AWS-Managed Runtime Bias

The current design favors managed AWS services over self-operated long-running platforms.

That shows up as:

- ECS Fargate for the continuously running Galexie process and one-time backfill tasks
- AWS Lambda for event-driven processing and API handlers
- RDS PostgreSQL for relational storage
- API Gateway and CloudFront for public delivery
- Secrets Manager, CloudWatch, and X-Ray for operational concerns

This keeps the runtime model operationally narrow and aligned with the serverless/event-
driven shape of the product.

### 2.3 Event-Driven Ingestion Path

Infrastructure is designed around an asynchronous ingestion chain:

1. Galexie streams canonical ledger data from Stellar peers
2. XDR files land in S3
3. S3 object creation triggers the Ledger Processor Lambda
4. parsed records are written to PostgreSQL
5. API and frontend read only from the explorer's own database

This separation is a core infrastructure assumption, not an implementation detail.

### 2.4 Progressive Reliability

Launch infrastructure is intentionally simpler than a long-term high-availability target.

The documented plan is:

- start in a single Availability Zone
- use Single-AZ RDS at launch
- expand to Multi-AZ and broader VPC topology only when SLA requirements justify it

The infrastructure should therefore be written so this progression is possible without
changing the overall architecture.

## 3. Target System Topology

### 3.1 End-to-End Topology

The infrastructure currently resolves into five runtime zones:

- Stellar network inputs
- ingestion components
- transient object storage
- explorer database storage
- public delivery layer

Logical flow:

```text
Stellar peers / history archives
  -> Galexie on ECS Fargate
  -> S3 ledger object storage
  -> Ledger Processor Lambda
  -> RDS PostgreSQL
  -> API Gateway + Lambda API
  -> CloudFront-served frontend clients
```

### 3.2 Public Traffic Path

Public user traffic should follow a simple path:

- the frontend is served through CloudFront as a static React application
- the frontend calls the public REST API through API Gateway
- public browser traffic is anonymous read-only and does not carry API keys
- API Gateway invokes Lambda-based Rust/axum handlers
- handlers read from RDS PostgreSQL only

No public client should connect directly to the database or to ingestion components.

### 3.3 Ingestion Traffic Path

Canonical chain data should follow a separate path:

- Galexie connects to Stellar peers through Captive Core
- the historical backfill task reads Stellar public history archives
- both live and historical ingestion produce `LedgerCloseMeta` XDR files in the same S3
  bucket format
- the same Ledger Processor Lambda handles both paths after S3 delivery

This infrastructure design avoids separate persistence pipelines for live data and backfill.

## 4. Deployment Model

### 4.1 AWS Account Model

All infrastructure runs in a dedicated AWS sub-account owned by Rumble Fish.

This matters because the infrastructure document assumes:

- isolated ownership of runtime resources and IAM boundaries
- infrastructure lifecycle controlled by the project team
- ability to redeploy the full stack without coordinating with an external platform owner

### 4.2 Launch Topology

At launch, the system is deployed in a single Availability Zone: `us-east-1a`.

The documented initial deployment model includes:

- one VPC
- public edge entry through CloudFront and API Gateway
- private subnet runtime for Lambda functions and RDS
- ECS Fargate Galexie running in the same VPC
- access to S3 through a VPC endpoint for Galexie

The design does not yet assume active-active regional redundancy or a multi-region failover
plan.

### 4.3 Public and Private Boundaries

The deployment sketch implies a public/private split:

- public-facing delivery components: CloudFront, API Gateway, Route 53
- private runtime components: Lambda API handlers, Ledger Processor (Indexer),
  RDS PostgreSQL, ECS Fargate workloads
- secret material accessed through Secrets Manager rather than baked into runtime images or
  application source

That split should remain stable even if the network layout expands later.

## 5. Managed Components

### 5.1 Ingestion Components

**Galexie process**

- runs on ECS Fargate as one continuous task for live ingestion
- connects to Stellar network peers via Captive Core
- emits one `LedgerCloseMeta` file per ledger close to S3

**Historical backfill task**

- runs on ECS Fargate as a batch/one-time process
- reads from Stellar public history archives
- writes the same XDR artifact format to the same S3 bucket so normal processing can be
  reused

### 5.2 Storage Components

**S3 bucket `stellar-ledger-data`**

- receives `LedgerCloseMeta` XDR files
- acts as transient object storage between Galexie and the Ledger Processor Lambda
- triggers the Ledger Processor via S3 object creation events
- is governed by lifecycle retention rules because replay and incident validation depend on
  short-term artifact availability

**RDS PostgreSQL**

- hosts the block explorer's owned relational schema
- stores explorer records and derived state
- serves as the only read source for the public API

### 5.3 Processing Components

**Lambda — Ledger Processor**

- is triggered by S3 PutObject events
- downloads and parses XDR using `@stellar/stellar-sdk`
- writes explorer records and derived state to RDS

### 5.4 API and Delivery Components

**Lambda — Rust/axum API handlers**

- serve all public REST endpoints
- read from RDS PostgreSQL only
- do not perform chain indexing or depend on external chain APIs for core responses

**API Gateway**

- exposes the public REST API
- provides request routing, throttling, request validation, and response caching
- may issue API keys for trusted non-browser consumers, but normal explorer browsing does
  not depend on browser-embedded keys

**AWS WAF**

- protects the public ingress layer attached to API Gateway and CloudFront
- applies managed rule sets, IP reputation filtering, and basic abuse controls
- provides browser-facing protection without relying on secrets in the SPA bundle

**CloudFront CDN**

- serves the React frontend
- caches static assets and documentation assets; API responses are not assumed to traverse
  CloudFront in the initial topology

**Swagger UI**

- served directly from the API (utoipa-swagger-ui `/api-docs` endpoint)
- no separate S3 bucket or CloudFront distribution needed

### 5.5 Operational Components

**Secrets Manager**

- stores database credentials and non-browser integration secrets

**CloudWatch + X-Ray**

- provide logs, metrics, dashboards, alarms, and distributed tracing

**GitHub Actions -> AWS CDK**

- provide the infrastructure deployment pipeline
- are the documented mechanism for infrastructure-as-code rollout

## 6. Networking and Security Boundary

### 6.1 Network Shape

The infrastructure sketch implies a VPC-centered runtime.

Current expected network shape:

- CloudFront, API Gateway, and AWS WAF form the public ingress layer
- application Lambdas and RDS live behind the private runtime boundary
- ECS Fargate Galexie runs inside the same VPC
- S3 access from Galexie is routed through a VPC endpoint

This keeps the database and ingestion workers out of the public network surface.

### 6.2 Secret Handling

Credential handling should remain centralized in Secrets Manager.

The documented design expects this for at least:

- database credentials
- non-browser integration secrets or keys

Browser-delivered frontend bundles do not contain API keys or other shared secrets.

Production transport and storage hardening baselines are also explicit:

- CloudFront and API Gateway serve public traffic over HTTPS/TLS
- production RDS storage is encrypted at rest with KMS-backed keys
- production S3 buckets use server-side encryption with KMS-backed keys
- application connections to the production database should require TLS

The architecture does not imply storing runtime secrets in source control, Lambda code, or
container images.

### 6.3 Public Exposure Rules

The infrastructure should keep public exposure narrow.

Publicly exposed surfaces are:

- CloudFront-hosted frontend delivery
- API Gateway-hosted REST API
- public DNS routing via Route 53
- API documentation served from utoipa-swagger-ui `/api-docs` endpoint

Those public surfaces should be protected by AWS WAF and API throttling. API keys, if
issued, are for trusted automation or partner use cases and are never required by the
browser application.

Non-public components should remain directly unreachable to external users.

### 6.4 External Dependency Boundary

The source design explicitly limits external runtime dependencies to read-only canonical
Stellar data sources:

- Stellar network peers for live data
- Stellar public history archives for one-time backfill

No other external API is required for core functionality.

## 7. Environments and Scalability

### 7.1 Environment Model

The infrastructure design defines three environments:

- **Development** using local PostgreSQL for local and CI workflows
- **Staging** using a separate RDS instance with testnet data
- **Production** using mainnet data on production RDS

The staging environment exists to validate infrastructure and runtime behavior before
production rollout.

Production is the public-service baseline. Staging should mirror topology and integration
shape, but not necessarily production-sized limits or retention defaults. The staging web
frontend is expected to be password-protected at the edge layer rather than exposed as a
fully public site.

### 7.2 Scaling Model

The source design documents scaling by component rather than a single platform-wide
mechanism.

Current expectations:

- API Lambda scales on demand, up to the documented concurrency tier
- Ledger Processor Lambda scales per S3-triggered ledger file
- CloudFront scales automatically
- PostgreSQL uses RDS Proxy by default for connection pooling
- materialized views are part of the default database scale/read strategy
- a read replica is introduced only when primary CPU passes the documented threshold

### 7.3 Availability Progression

High availability is staged rather than assumed at launch.

Documented progression:

- start with Single-AZ RDS and a single-AZ deployment footprint
- add Multi-AZ and broader VPC expansion when SLA exceeds 99.9%

This is important because the infrastructure doc should not imply higher availability than
what the source design currently commits to.

### 7.4 Environment-Specific Limits and Protections

The source design should be read as defining production baselines unless stated otherwise.
Staging and production should not share identical operational limits.

**Staging profile**

- lower API concurrency and throttling ceilings than production
- smaller API Gateway cache allocation and shorter cache TTLs where cache is enabled
- lower-cost database/storage sizing suitable for testnet validation rather than public load
- shorter log, trace, and transient-artifact retention windows; staging replay artifacts
  in `stellar-ledger-data` should be kept for at least 7 days
- password protection for the staging web frontend at the CloudFront/edge layer; optional
  additional controls such as IP allowlists or reduced DNS discoverability when needed
- alerting tuned to catch regressions quickly without mirroring every production paging rule

**Production profile**

- public-internet availability with WAF and API throttling sized for anonymous browser
  traffic
- response caching, Lambda concurrency, and database sizing tuned for real public demand
- longer operational retention for logs, traces, and replay-relevant artifacts; production
  replay artifacts in `stellar-ledger-data` should be kept for at least 30 days
- automated RDS backups, point-in-time recovery, and deletion protection enabled on the
  production database
- KMS-backed encryption for production RDS and S3, with TLS enforced on public ingress and
  production database connections
- full paging and SLA-oriented alert thresholds

Exact values should live in environment-specific CDK configuration rather than being hard-
coded into the document, but the separation of profiles is part of the architecture.

## 8. Observability and Operations

### 8.1 Monitoring Surface

The infrastructure design already defines a monitoring baseline.

CloudWatch dashboards should expose at least:

- Galexie S3 file freshness
- Ledger Processor duration
- Ledger Processor error rate
- API latency across p50/p95/p99
- RDS CPU and connection metrics
- highest indexed ledger sequence versus network tip

### 8.2 Alerting Surface

The documented alarms are:

- Galexie ingestion lag when S3 file timestamps are more than 60 seconds behind ledger close
- Ledger Processor error rate above 1% of Lambda invocations
- RDS CPU above 70% sustained for 5 minutes
- RDS free storage below 20% remaining
- API Gateway 5xx rate above 0.5% of requests

These values are the production baseline. Staging should preserve the same alert categories,
but may use lower-volume thresholds, shorter retention, and non-paging notification rules to
match its lower traffic and lower cost profile.

### 8.3 Recovery Assumptions

The source design documents specific operational recovery assumptions:

- Galexie is checkpoint-aware and resumes from the last exported ledger on restart
- Lambda retries S3-triggered processing automatically
- failed ledger files remain in S3 and can be replayed by re-triggering the Lambda
- schema migrations run before new Lambda code deployment in the CI/CD pipeline
- protocol upgrades are handled by updating XDR type support in `@stellar/stellar-sdk`

These assumptions connect runtime infrastructure directly to safe ingestion operations.

## 9. Delivery Model and Workspace Boundary

### 9.1 Infrastructure as Code Boundary

The documented infrastructure direction is AWS CDK written in TypeScript.

Within the current workspace structure, that boundary maps to:

- `infra/aws-cdk` for infrastructure definitions
- application packages under `apps/*` as runtime artifacts deployed by the infrastructure

The infrastructure doc should therefore be read as the target design input for the future
CDK stack, not as a claim that the full stack already exists in the repository.

### 9.2 CI/CD Model

The source design defines the delivery path as:

- GitHub Actions
- infrastructure deployment through `cdk deploy`
- environment parity work across staging and production

Infrastructure rollout is therefore part of the product delivery model, not a manual-only
operations process.

### 9.3 Public-Repo Configuration Model

Because the repository is public, infrastructure configuration must be split between
non-secret config committed to git and secrets resolved outside the repository.

Safe-to-commit infrastructure config includes:

- environment names, AWS region, and account/stack identifiers
- instance classes, cache sizes, retention periods, and scaling thresholds
- public domain names and routing structure
- feature flags and non-sensitive deployment toggles
- secret references such as parameter names, secret names, or ARNs, but not secret values

The repository should not contain:

- database passwords, staging web passwords, API keys, webhook secrets, or private keys
- `.env.prod`, `.env.staging`, or similar files containing real secret values
- copied secret payloads inside CDK context files, TypeScript constants, or GitHub workflow
  YAML

Expected secure configuration model:

- non-secret environment config lives in `infra/aws-cdk/config/*`
- real secret values live in AWS Secrets Manager or SSM Parameter Store SecureString
- CDK consumes secret references, not hard-coded secret values
- runtime workloads (Lambda, ECS) read only the specific secrets they need through IAM
- the staging web password is stored as a secret and referenced by the edge protection
  mechanism rather than committed to the repository

### 9.4 CI/CD Credentials and Deployment Access

For a public repository, CI/CD authentication should avoid long-lived AWS credentials stored
in GitHub secrets.

Preferred model:

- GitHub Actions uses OIDC to assume AWS roles at deploy time
- staging and production use separate AWS roles and separate environment protections
- IAM permissions remain least-privilege and environment-scoped
- deployment workflows may know which secret to reference, but not embed the secret value

This keeps the public repository redeployable without turning the repository itself into a
secret distribution channel.

### 9.5 Open-Source Redeployability

The main design explicitly assumes the full stack can be redeployed by third parties.

That means the infrastructure design should remain:

- self-contained
- AWS-account reproducible
- free of hidden dependency on internal-only external services for core runtime behavior

### 9.6 Current Workspace State

The repository currently documents the intended infrastructure shape and reserves
`infra/aws-cdk` as the infrastructure boundary, but does not yet contain the final deployed
runtime implementation.

That is expected. This document should serve as the detailed reference for future
infrastructure implementation planning, while
[`technical-design-general-overview.md`](../technical-design-general-overview.md) remains
the primary source of truth.
