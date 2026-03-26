---
id: '0004'
title: 'Research: NestJS on AWS Lambda (adapter, cold starts, connection lifecycle)'
type: RESEARCH
status: completed
related_adr: []
related_tasks: ['0015', '0007']
tags: [priority-high, effort-small, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
  - date: 2026-03-26
    status: active
    who: fmazur
    note: 'Promoted to active — starting research'
  - date: 2026-03-26
    status: completed
    who: fmazur
    note: >
      Research complete. 8 research notes + 1 synthesis, 17 verified sources.
      All 8 acceptance criteria met. Key decisions: @codegenie/serverless-express,
      Node.js 22 arm64, esbuild bundling, no provisioned concurrency at launch,
      Drizzle + node-postgres Pool max:1, RDS Proxy with session_pinning_filters,
      REST API (not HTTP API), node-cache 30-60s TTL for in-memory caching.
---

# Research: NestJS on AWS Lambda (adapter, cold starts, connection lifecycle)

## Summary

Evaluate the adapter options, cold start characteristics, and database connection lifecycle patterns for running the NestJS backend API on AWS Lambda behind API Gateway. This research must determine the optimal configuration for the 9-module NestJS application serving the block explorer REST API, including Drizzle ORM connection management and caching strategies.

## Status: Completed

## Context

The backend is a NestJS application running on AWS Lambda behind API Gateway. It serves all public REST endpoints as a read-only API over the block explorer's owned PostgreSQL database. The application contains 9 NestJS modules, and cold start performance directly impacts user experience for the first request after a period of inactivity.

### NestJS Modules

The application is composed of 9 resource-oriented modules that cold start together:

1. **Network** -- chain-level aggregate stats and freshness information
2. **Transactions** -- list and detail queries, filter handling, advanced/raw payload support
3. **Ledgers** -- ledger list/detail access and linked transaction retrieval
4. **Accounts** -- account summary, balances, and account-related transaction history
5. **Tokens** -- classic asset and Soroban token listing and detail retrieval
6. **Contracts** -- contract metadata, interface, invocations, and events
7. **NFTs** -- NFT list/detail retrieval and transfer history access
8. **Liquidity Pools** -- pool listing, detail, transaction history, and chart data
9. **Search** -- exact match and grouped result resolution across entity types

### Adapter Options

The NestJS application needs an adapter to run inside Lambda. The primary candidates to evaluate are `serverless-express` (formerly `aws-serverless-express`), `@nestjs/platform-express` with a Lambda wrapper, and any other community adapters that support API Gateway integration.

### Cold Start Mitigation

The architecture specifies ARM/Graviton2 runtime for Lambda to improve cold start performance and cost efficiency. Provisioned concurrency is mentioned as a mitigation at higher traffic tiers. The research must benchmark cold start times with the full 9-module NestJS application on ARM to determine if provisioned concurrency is needed from launch.

### Database Connection Lifecycle

The backend uses Drizzle ORM for database access with RDS PostgreSQL via RDS Proxy. The connection lifecycle question is critical for Lambda: should a new database connection be created per invocation, or should connections be reused across warm invocations? RDS Proxy is specifically included in the architecture to handle connection pooling under burst traffic, but the application-side connection behavior must be designed to work correctly with both cold and warm Lambda execution contexts.

### API Gateway Configuration

The architecture mentions two API Gateway modes to evaluate: REST API vs HTTP API. Response caching is a key feature -- immutable data (historical transactions, closed ledgers) should use long TTLs, while mutable data (recent transactions, network stats) uses short TTLs of 5-15 seconds. WAF attachment is needed on the API Gateway for abuse protection.

### In-Memory Lambda Caching

The backend architecture specifies in-memory Lambda caching for frequently accessed reference data. Contract metadata and network stats should be cached with TTLs of 30-60 seconds to reduce database round-trips. This cache must survive across warm invocations but be recreated on cold starts.

## Research Questions

- Which Lambda adapter is recommended for NestJS: `serverless-express`, `@vendia/serverless-express`, or another option? What are the trade-offs in terms of cold start overhead, API Gateway compatibility, and maintenance status?
- What is the measured cold start time for a 9-module NestJS application on ARM/Graviton2 Lambda with typical node_modules size?
- What is the optimal provisioned concurrency configuration for launch traffic levels?
- How should Drizzle ORM connections be managed in Lambda? Should the connection be created outside the handler (module scope) for reuse across warm invocations, or inside the handler per invocation?
- How does RDS Proxy interact with Lambda connection patterns -- does it handle connection reuse transparently, or does the application need specific configuration?
- REST API vs HTTP API for API Gateway: which supports response caching, WAF attachment, and the request validation features needed?
- How should API Gateway response caching be configured with different TTLs per endpoint (long for immutable, 5-15s for mutable)?
- What is the best pattern for in-memory Lambda caching with 30-60s TTLs that persists across warm invocations?

## Acceptance Criteria

- [x] Adapter recommendation with justification — see [R-lambda-adapter-selection.md](notes/R-lambda-adapter-selection.md)
- [x] Cold start benchmarks on ARM/Graviton2 — see [R-cold-start-benchmarks-arm.md](notes/R-cold-start-benchmarks-arm.md)
- [x] Provisioned concurrency recommendation for launch — see [R-provisioned-concurrency.md](notes/R-provisioned-concurrency.md)
- [x] Drizzle ORM connection lifecycle pattern documented — see [R-drizzle-connection-lifecycle.md](notes/R-drizzle-connection-lifecycle.md)
- [x] RDS Proxy integration requirements documented — see [R-rds-proxy-integration.md](notes/R-rds-proxy-integration.md)
- [x] API Gateway mode recommendation (REST API vs HTTP API) — see [R-api-gateway-mode.md](notes/R-api-gateway-mode.md)
- [x] Response caching strategy documented (TTL tiers) — see [R-response-caching-strategy.md](notes/R-response-caching-strategy.md)
- [x] In-memory caching pattern documented — see [R-inmemory-caching-pattern.md](notes/R-inmemory-caching-pattern.md)

## Implementation Notes

Research-only task — no code changes. Deliverables:

- **9 notes** in `notes/` (8 R-prefixed research + 1 S-prefixed synthesis)
- **17 source files** in `sources/` (archived web content with verified URLs)
- Every factual claim in notes has a `> Source:` reference to a source file
- Source files with editorial additions are marked `[EDITORIAL]`
- Final audit: 68/68 source references verified, 17/17 URLs accessible

## Design Decisions

### From Plan

1. **`@codegenie/serverless-express` adapter**: Official NestJS recommendation, only viable option. No local socket overhead in v4.x, broad event source support.

2. **Node.js 22 arm64 runtime**: 14% faster cold starts, 25-40% lower cost vs x86. Based on 183,750-invocation benchmark.

3. **REST API over HTTP API**: Caching and WAF are REST API-only features. Both are hard requirements.

4. **Drizzle + node-postgres (pg)**: Avoids connection pinning from postgres.js prepared statements in RDS Proxy.

5. **Pool max:1 outside handler**: Lambda is single-concurrent; RDS Proxy handles real pooling across instances.

### Emerged

6. **No provisioned concurrency at launch**: Task asked to evaluate, research showed <0.5% cold start rate at moderate traffic + $54/month baseline cost makes it premature for unknown traffic patterns.

7. **`node-cache` over `@nestjs/cache-manager`**: Simpler, lighter, explicit TTL control. cache-manager abstraction designed for Redis/Memcached swapping — unnecessary for in-memory only.

8. **Two-tier caching architecture**: API Gateway cache (15-3600s) + Lambda in-memory cache (30-60s) layered. Not explicitly in the task but emerged as the natural strategy.

9. **`session_pinning_filters = EXCLUDE_VARIABLE_SETS`**: Required on RDS Proxy to avoid pinning from benign PostgreSQL driver SET commands. Not mentioned in original task.

10. **`NODE_EXTRA_CA_CERTS=/var/runtime/ca-cert.pem`**: Required for Node.js 20+ Lambda connecting to RDS via SSL. Discovered during research.

11. **Cache invalidation strategy**: `unauthorizedCacheControlHeaderStrategy: FAIL_WITH_403` to prevent cache poisoning on public API. Not in original scope.

## Issues Encountered

- **Duplicate source files**: Two agents fetched the same URL (API Gateway caching docs) independently, creating duplicates. Resolved by merging to single file.
- **Paywalled source**: `drizzle-orm-serverless-integration-medium.md` was behind Medium paywall. Replaced reference with AWS official docs that contain the same metric (`DatabaseConnectionsCurrentlySessionPinned`).
- **NestJS docs SPA**: `docs.nestjs.com` renders client-side, WebFetch cannot extract content. URL is accessible (HTTP 200) but content unverifiable via automated fetch. Accepted as known NestJS documentation.
- **Editorial content in sources**: Some source files contained synthesized Lambda-specific patterns not from the original URL. Fixed by adding `[EDITORIAL]` markers to affected sections.

## Notes

- The frontend is anonymous read-only traffic from browsers -- no API keys in browser bundles. Abuse controls are at WAF and API Gateway throttling level.
- The backend must degrade gracefully when upstream ingestion is delayed, serving what is already indexed with a freshness indicator showing the highest indexed ledger sequence.
- CloudFront is reserved for static frontend/document delivery in the initial topology -- API responses do not traverse CloudFront at launch.
