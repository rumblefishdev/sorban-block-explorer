---
id: '0004'
title: 'Research: NestJS on AWS Lambda (adapter, cold starts, connection lifecycle)'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0015', '0007']
tags: [priority-high, effort-small, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
---

# Research: NestJS on AWS Lambda (adapter, cold starts, connection lifecycle)

## Summary

Evaluate the adapter options, cold start characteristics, and database connection lifecycle patterns for running the NestJS backend API on AWS Lambda behind API Gateway. This research must determine the optimal configuration for the 9-module NestJS application serving the block explorer REST API, including Drizzle ORM connection management and caching strategies.

## Status: Backlog

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

- [ ] Adapter recommendation with justification (cold start impact, compatibility, maintenance)
- [ ] Cold start benchmarks on ARM/Graviton2 with representative NestJS module count
- [ ] Provisioned concurrency recommendation for launch
- [ ] Drizzle ORM connection lifecycle pattern documented for Lambda (create/reuse semantics)
- [ ] RDS Proxy integration requirements documented
- [ ] API Gateway mode recommendation (REST API vs HTTP API) with feature comparison
- [ ] Response caching strategy documented (TTL tiers per endpoint category)
- [ ] In-memory caching pattern documented for warm Lambda reuse

## Notes

- The frontend is anonymous read-only traffic from browsers -- no API keys in browser bundles. Abuse controls are at WAF and API Gateway throttling level.
- The backend must degrade gracefully when upstream ingestion is delayed, serving what is already indexed with a freshness indicator showing the highest indexed ledger sequence.
- CloudFront is reserved for static frontend/document delivery in the initial topology -- API responses do not traverse CloudFront at launch.
