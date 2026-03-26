---
prefix: R
title: 'Drizzle ORM Connection Lifecycle in Lambda'
status: mature
spawned_from: '0004'
---

# R: Drizzle ORM Connection Lifecycle in Lambda

## Recommendation: `node-postgres` (pg) with Pool outside handler, max: 1

### Driver Choice: `node-postgres` over `postgres.js`

| Factor                  | node-postgres (pg)                       | postgres.js                                          |
| ----------------------- | ---------------------------------------- | ---------------------------------------------------- |
| RDS Proxy compatibility | Works out of the box                     | Requires `prepare: false`                            |
| Prepared statements     | No binary prepared statements by default | Uses prepared statements by default (causes pinning) |
| Connection pinning risk | Low                                      | High without `prepare: false`                        |
| Ecosystem maturity      | Older, battle-tested                     | Newer, fewer Lambda references                       |

> Source: [drizzle-orm-pg-connection-docs.md](../sources/drizzle-orm-pg-connection-docs.md), "Key Differences Between Drivers" section (postgres.js uses prepared statements by default); [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "Pinning" section (prepared statements cause pinning)

**Why `node-postgres`:** Binary prepared statements in `postgres.js` cause **connection pinning** in RDS Proxy, which defeats the purpose of the proxy's connection multiplexing. While `prepare: false` works around this, using `node-postgres` avoids the problem entirely.

> Source: [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "Pinning" section — "Prepared statements (PostgreSQL binary protocol)" listed as pinning cause

### Connection Pattern

Initialize the Pool **outside** the Lambda handler for reuse across warm invocations:

```typescript
import { drizzle } from 'drizzle-orm/node-postgres';
import { Pool } from 'pg';

// Outside handler — persists across warm invocations
const pool = new Pool({
  host: process.env.RDS_PROXY_HOST,
  database: process.env.DB_NAME,
  user: process.env.DB_USER,
  password: process.env.DB_PASSWORD,
  port: 5432,
  max: 1, // 1 connection per Lambda instance
  min: 0, // No minimum maintained connections
  idleTimeoutMillis: 120000, // 2 min idle before release
  connectionTimeoutMillis: 10000,
  ssl: { rejectUnauthorized: false },
});

const db = drizzle({ client: pool });
```

> Source: [drizzle-orm-pg-connection-docs.md](../sources/drizzle-orm-pg-connection-docs.md), "Using an Existing Pool" section; [lambda-db-connection-reuse-jeremydaly.md](../sources/lambda-db-connection-reuse-jeremydaly.md), "Recommended Pool Configuration for Lambda"

### Why `max: 1`

Each Lambda instance handles one request at a time. A pool size of 1 is sufficient. RDS Proxy handles real connection pooling across all Lambda instances — e.g., 500 Lambda clients might use just 20 persistent database connections through RDS Proxy.

> Source: [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "Connection Multiplexing" section — "500 Lambda clients might use just 20 persistent database connections"
> Source: [node-postgres-lambda-rds-proxy-best-practices.md](../sources/node-postgres-lambda-rds-proxy-best-practices.md), "Pool Sizing for Lambda" section

### `callbackWaitsForEmptyEventLoop`

When using callback-based resolution, set `context.callbackWaitsForEmptyEventLoop = false`. This prevents Lambda from waiting for the connection pool to drain before returning the response. Without this, Lambda would time out or add unnecessary latency.

> Source: [lambda-db-connection-reuse-jeremydaly.md](../sources/lambda-db-connection-reuse-jeremydaly.md), "Critical Configuration" section

**Note:** With `@codegenie/serverless-express` in default `PROMISE` resolution mode and async/await handlers, this is less critical — but still recommended when using the pool pattern.

### Freeze/Thaw Behavior

Lambda containers are frozen between invocations and thawed for reuse. TCP sockets may become stale after a long freeze:

- `idleTimeoutMillis: 120000` handles stale connections by closing them after 2 minutes of inactivity
- RDS Proxy is more resilient to freeze/thaw because it maintains real DB connections server-side
- The pool automatically creates a new connection if the frozen one is stale

> Source: [node-postgres-lambda-rds-proxy-best-practices.md](../sources/node-postgres-lambda-rds-proxy-best-practices.md), "The Freeze/Thaw Problem" section

### Warm vs Cold Performance

- **Cold start:** Pool creation + first connection (~200ms+ for connection establishment)
- **Warm invocation:** Connection reuse, 4-20ms query overhead

> Source: [lambda-db-connection-reuse-jeremydaly.md](../sources/lambda-db-connection-reuse-jeremydaly.md), "Performance Results" section

### NestJS Integration

In NestJS, the Drizzle module should be configured as a global provider with the pool initialized at module scope. The connection survives across warm invocations because NestJS itself is cached via the `cachedServer` pattern — the entire app instance (including Drizzle) persists.

### Query Patterns

For our read-only API:

- Use `pool.query()` (via Drizzle) directly — no need for manual client acquisition
- For concurrent queries within a single request: use `Promise.all()` with separate `pool.query()` calls
- Our read-only API has no transactions, so pinning from `BEGIN/COMMIT` is not a concern

> Source: [node-postgres-lambda-rds-proxy-best-practices.md](../sources/node-postgres-lambda-rds-proxy-best-practices.md), "Per-Request Approach" and "Concurrent Queries" sections
