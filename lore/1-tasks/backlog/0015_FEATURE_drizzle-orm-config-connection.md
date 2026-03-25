---
id: '0015'
title: 'Drizzle ORM configuration and connection setup'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0004', '0007']
tags: [priority-high, effort-medium, layer-database]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Drizzle ORM configuration and connection setup

## Summary

Set up Drizzle ORM as the database access layer for the Stellar Block Explorer. This includes connection configuration, environment-aware connection string resolution, RDS Proxy integration, and Lambda-optimized connection lifecycle management. Two application consumers depend on this: apps/api (read-heavy REST API) and apps/indexer (write-heavy Ledger Processor Lambda).

## Status: Backlog

**Current state:** Not started.

## Context

The block explorer uses PostgreSQL (RDS) as its sole data store. Two distinct Lambda-based applications connect to it:

1. **apps/api** -- the public NestJS REST API Lambda. Read-heavy workload serving explorer queries.
2. **apps/indexer** -- the Ledger Processor Lambda triggered by S3 PutObject events. Write-heavy workload that parses LedgerCloseMeta XDR and persists explorer records.

Both applications connect to RDS through **RDS Proxy**, which is the default for ALL Lambda-to-DB connections. RDS Proxy manages connection pools to prevent exhaustion under burst traffic from concurrent Lambda invocations.

### Environment Matrix

| Environment | Database              | Notes                                                   |
| ----------- | --------------------- | ------------------------------------------------------- |
| dev         | Local PostgreSQL      | Direct connection, no proxy, no TLS                     |
| staging     | Separate RDS instance | Testnet data, RDS Proxy enabled                         |
| production  | Mainnet RDS instance  | RDS Proxy enabled, TLS required, KMS encryption at rest |

### Connection String Management

- Connection strings are stored in **AWS Secrets Manager**, one per environment.
- Lambda functions retrieve the connection string from Secrets Manager at cold-start time.
- No credentials are baked into application source, container images, or environment variable literals.

### Production Security

- TLS is required for all production database connections (`rds.force_ssl = 1` parameter group setting).
- RDS storage is encrypted at rest with KMS-backed keys.
- RDS Proxy handles IAM-authenticated or Secrets Manager-authenticated connection pooling.

### Lambda Connection Lifecycle

- Database connections are established at **module level** (outside the handler function) so they are reused across warm Lambda invocations.
- On cold start, a new connection is established and cached for the lifetime of the execution environment.
- RDS Proxy absorbs the connection management complexity, so individual Lambda instances do not need to implement their own pool logic.

## Implementation Plan

### Step 1: Drizzle ORM package setup

Install Drizzle ORM and Drizzle Kit as workspace dependencies. Configure the drizzle.config.ts at the appropriate workspace level (likely a shared database library under libs/).

### Step 2: Connection factory with environment resolution

Create a connection factory that:

- Reads the database connection string from Secrets Manager (staging/production) or local environment variable (dev).
- Returns a configured Drizzle client instance.
- Supports the module-level caching pattern for Lambda warm reuse.

### Step 3: RDS Proxy configuration

Ensure the connection factory targets the RDS Proxy endpoint for staging and production environments. The proxy endpoint replaces the direct RDS endpoint in the connection string.

### Step 4: TLS configuration for production

Add TLS/SSL options to the connection configuration when running in production. Ensure the connection is rejected if TLS cannot be established (matching rds.force_ssl = 1).

### Step 5: Integration with apps/api and apps/indexer

Wire the Drizzle connection factory into both application entry points:

- apps/api: NestJS module that provides the Drizzle client as an injectable dependency.
- apps/indexer: Lambda handler module that initializes the connection at module scope.

### Step 6: Local development configuration

Provide a docker-compose or local PostgreSQL configuration for the dev environment. Ensure drizzle.config.ts supports running Drizzle Kit commands (push, generate, migrate) against local PostgreSQL.

## Acceptance Criteria

- [ ] Drizzle ORM and Drizzle Kit are installed and configured in the workspace
- [ ] Connection factory resolves credentials from Secrets Manager in staging/production
- [ ] Connection factory uses local PostgreSQL config in dev environment
- [ ] RDS Proxy endpoint is used for all Lambda-to-DB connections in staging and production
- [ ] TLS is enforced on production connections
- [ ] Module-level connection reuse works correctly in Lambda (warm invocation reuse)
- [ ] apps/api can obtain a Drizzle client through NestJS dependency injection
- [ ] apps/indexer can obtain a Drizzle client at module scope
- [ ] Connection works end-to-end in local dev with a local PostgreSQL instance

## Notes

- The Drizzle schema definitions themselves are defined in subsequent tasks (0016-0020). This task focuses on the ORM configuration and connection plumbing only.
- RDS Proxy is non-negotiable for Lambda workloads. Direct RDS connections from Lambda lead to connection exhaustion under burst concurrency.
- The connection factory should be placed in a shared library so both apps/api and apps/indexer can import it without duplication.
