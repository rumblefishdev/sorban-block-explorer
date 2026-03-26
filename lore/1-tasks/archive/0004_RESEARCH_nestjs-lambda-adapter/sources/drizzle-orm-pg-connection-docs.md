---
url: 'https://orm.drizzle.team/docs/get-started-postgresql'
title: 'Drizzle ORM - PostgreSQL Connection Setup'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
editorial_note: >
  Sections marked [EDITORIAL] below are synthesized from other sources
  (node-postgres issue #3016, Jeremy Daly article, AWS RDS Proxy docs),
  not from the Drizzle ORM page itself. The original page covers driver
  setup and basic connection patterns only.
---

# Drizzle ORM - PostgreSQL Connection Setup

## Overview

Drizzle ORM provides native PostgreSQL support through two primary drivers: `node-postgres` and `postgres.js`. The documentation assumes familiarity with database connection basics and the respective driver fundamentals.

## Key Differences Between Drivers

**node-postgres (`pg`):**

- Optional `pg-native` installation enhances performance by ~10%
- Supports per-query type parsers without global patching
- Greater flexibility for custom type handling
- Does **not** use binary prepared statements by default [EDITORIAL: "(safer with RDS Proxy)" added — not stated on Drizzle page, inferred from RDS Proxy pinning docs]

**postgres.js:**

- Uses **prepared statements by default** [EDITORIAL: "requires `prepare: false` for RDS Proxy compatibility" — Drizzle page says "you may need to opt out" without mentioning RDS Proxy specifically]
- Different connection initialization approach
- May require configuration adjustments in AWS environments

## node-postgres Setup

### Installation

```bash
npm i drizzle-orm pg
npm i -D drizzle-kit @types/pg
```

### Basic Connection

```typescript
import { drizzle } from 'drizzle-orm/node-postgres';

const db = drizzle(process.env.DATABASE_URL);
const result = await db.execute('select 1');
```

### Connection with SSL Configuration

```typescript
import { drizzle } from 'drizzle-orm/node-postgres';

const db = drizzle({
  connection: {
    connectionString: process.env.DATABASE_URL,
    ssl: true,
  },
});

const result = await db.execute('select 1');
```

### Using an Existing Pool

[EDITORIAL: The Lambda-specific pool configuration below (max:1, idleTimeoutMillis, callbackWaitsForEmptyEventLoop, handler pattern) is synthesized from node-postgres issue #3016 and Jeremy Daly's article. The Drizzle docs page only shows basic `drizzle({ client: pool })` pattern without Lambda-specific settings.]

This pattern is critical for Lambda — initialize the pool outside the handler for connection reuse:

```typescript
import { drizzle } from 'drizzle-orm/node-postgres';
import { Pool } from 'pg';

// Initialize outside handler for Lambda reuse
const pool = new Pool({
  connectionString: process.env.DATABASE_URL,
  max: 1, // 1 connection per Lambda instance
  idleTimeoutMillis: 120000, // 2 minutes
  connectionTimeoutMillis: 10000,
});

const db = drizzle({ client: pool });

// Lambda handler
export const handler = async (event: any, context: any) => {
  context.callbackWaitsForEmptyEventLoop = false;
  return await db.select().from(users);
};
```

### Accessing Underlying Client

```typescript
const pool = db.$client; // Access the underlying Pool instance
```

## postgres.js Setup

### Installation

```bash
npm i drizzle-orm postgres
npm i -D drizzle-kit
```

### Basic Connection

```typescript
import { drizzle } from 'drizzle-orm/postgres-js';

const db = drizzle(process.env.DATABASE_URL);
const result = await db.execute('select 1');
```

### Connection with SSL Configuration

```typescript
import { drizzle } from 'drizzle-orm/postgres-js';

const db = drizzle({
  connection: {
    url: process.env.DATABASE_URL,
    ssl: true,
  },
});

const result = await db.execute('select 1');
```

### Using Existing Client (Lambda / RDS Proxy)

[EDITORIAL: The RDS Proxy commentary and `max: 1` are synthesized. The Drizzle page shows `prepare: false` as an option but does not mention RDS Proxy or connection pinning explicitly.]

```typescript
import { drizzle } from 'drizzle-orm/postgres-js';
import postgres from 'postgres';

// IMPORTANT: prepare: false is required for RDS Proxy compatibility
const queryClient = postgres(process.env.DATABASE_URL!, {
  prepare: false, // Disables prepared statements — prevents connection pinning in RDS Proxy
  max: 1, // 1 connection per Lambda instance
});

const db = drizzle({ client: queryClient });
```

## Serverless / Edge Support

Drizzle natively supports serverless runtimes:

```typescript
// Neon HTTP (serverless HTTP protocol, no persistent connections)
import { drizzle } from 'drizzle-orm/neon-http';
const db = drizzle(process.env.DATABASE_URL);

// Vercel Postgres
import { drizzle } from 'drizzle-orm/vercel-postgres';
const db = drizzle();

// Cloudflare D1
import { drizzle } from 'drizzle-orm/d1';
const db = drizzle({ connection: env.DB });
```

For traditional PostgreSQL in AWS Lambda, use `node-postgres` or `postgres.js` with the pool patterns above.

## Connection URL Format

```
postgresql://username:password@hostname:port/database
```

## Driver Architecture

```
Drizzle query → Database driver (pg / postgres.js) → SQL execution → Database → Results
```

Access the underlying driver client via `db.$client`:

```typescript
const pool = db.$client; // Returns the Pool or postgres.js client
```

## AWS Lambda Summary

[EDITORIAL: This summary table is synthesized from multiple sources (Drizzle docs, node-postgres #3016, RDS Proxy docs). It does not appear on the Drizzle ORM page.]

| Driver               | RDS Proxy Compatible                           | Lambda Pattern                                              |
| -------------------- | ---------------------------------------------- | ----------------------------------------------------------- |
| `node-postgres` (pg) | Yes (no binary prepared statements by default) | `Pool({ max: 1 })` outside handler                          |
| `postgres.js`        | Yes, with `prepare: false`                     | `postgres(url, { prepare: false, max: 1 })` outside handler |

## Next Steps

**Schema Management:**

- SQL schema declaration
- PostgreSQL data types
- Indexes and constraints
- Database views, schemas, sequences, extensions

**Data Querying:**

- Relational queries
- Select, insert, update, delete operations
- Filtering and joins
