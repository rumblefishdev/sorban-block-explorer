---
prefix: S
title: 'Synthesis: NestJS Lambda Configuration Recommendations'
status: mature
spawned_from: '0004'
---

# S: NestJS Lambda Configuration — Final Recommendations

## Executive Summary

| Area                        | Recommendation                                                    | Key Source                                                             |
| --------------------------- | ----------------------------------------------------------------- | ---------------------------------------------------------------------- |
| **Adapter**                 | `@codegenie/serverless-express`                                   | [R-lambda-adapter-selection.md](R-lambda-adapter-selection.md)         |
| **Architecture**            | Mono-lambda (all 9 modules in one function)                       | [R-lambda-adapter-selection.md](R-lambda-adapter-selection.md)         |
| **Runtime**                 | Node.js 22 on arm64 (Graviton2)                                   | [R-cold-start-benchmarks-arm.md](R-cold-start-benchmarks-arm.md)       |
| **Bundler**                 | esbuild (40-90% cold start reduction)                             | [R-cold-start-benchmarks-arm.md](R-cold-start-benchmarks-arm.md)       |
| **Provisioned Concurrency** | Not at launch — revisit when traffic patterns stabilize           | [R-provisioned-concurrency.md](R-provisioned-concurrency.md)           |
| **DB Driver**               | Drizzle + `node-postgres` (pg), Pool max:1 outside handler        | [R-drizzle-connection-lifecycle.md](R-drizzle-connection-lifecycle.md) |
| **Connection Proxy**        | RDS Proxy with `session_pinning_filters`                          | [R-rds-proxy-integration.md](R-rds-proxy-integration.md)               |
| **API Gateway**             | REST API (not HTTP API) — caching + WAF required                  | [R-api-gateway-mode.md](R-api-gateway-mode.md)                         |
| **Response Cache**          | 3-tier TTL: 3600s immutable / 60-300s semi-stable / 5-15s mutable | [R-response-caching-strategy.md](R-response-caching-strategy.md)       |
| **In-Memory Cache**         | `node-cache` with 30-60s TTL for reference data                   | [R-inmemory-caching-pattern.md](R-inmemory-caching-pattern.md)         |

## Cold Start Budget

Expected cold start for production deployment:

| Component                              | Duration       |
| -------------------------------------- | -------------- |
| Node.js 22 arm64 runtime init          | ~129ms         |
| esbuild bundle load + NestJS bootstrap | ~200-400ms     |
| First DB connection via RDS Proxy      | ~100-200ms     |
| **Total estimated cold start**         | **~400-700ms** |

- Cold starts affect <0.5% of requests at moderate traffic
- Warm response time: ~70ms average

## Caching Architecture (Two-Tier)

```
Client → API Gateway Cache (REST API)
         ├─ HIT → return cached response (no Lambda invocation)
         └─ MISS → Lambda
                   ├─ In-Memory Cache (node-cache, 30-60s TTL)
                   │  ├─ HIT → return from memory (no DB query)
                   │  └─ MISS → Database (via RDS Proxy)
                   └─ Response → API Gateway Cache (stored for next request)
```

## Lambda Handler Blueprint

```typescript
import { NestFactory } from '@nestjs/core';
import serverlessExpress from '@codegenie/serverless-express';
import { Handler } from 'aws-lambda';
import { AppModule } from './app.module';

let server: Handler;

async function bootstrap(): Promise<Handler> {
  const app = await NestFactory.create(AppModule);
  await app.init();
  const expressApp = app.getHttpAdapter().getInstance();
  return serverlessExpress({ app: expressApp });
}

export const handler: Handler = async (event, context, callback) => {
  server = server ?? (await bootstrap());
  return server(event, context, callback);
};
```

## Drizzle Connection Blueprint

```typescript
import { drizzle } from 'drizzle-orm/node-postgres';
import { Pool } from 'pg';

const pool = new Pool({
  host: process.env.RDS_PROXY_HOST,
  database: process.env.DB_NAME,
  user: process.env.DB_USER,
  password: process.env.DB_PASSWORD,
  port: 5432,
  max: 1,
  min: 0,
  idleTimeoutMillis: 120000,
  connectionTimeoutMillis: 10000,
  ssl: { rejectUnauthorized: false },
});

export const db = drizzle({ client: pool });
```

## Cost Optimization

1. **ARM64** saves 25-40% per invocation vs x86
2. **API Gateway caching** eliminates Lambda invocations for cached responses
3. **In-memory caching** reduces DB queries per Lambda instance
4. **No provisioned concurrency** avoids $54+/month baseline cost at launch
5. **RDS Proxy** handles connection pooling without application-side complexity
