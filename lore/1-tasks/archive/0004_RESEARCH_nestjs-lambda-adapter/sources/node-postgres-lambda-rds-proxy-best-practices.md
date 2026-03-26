---
url: 'https://github.com/brianc/node-postgres/issues/3016'
title: 'AWS Lambda + RDS Proxy and Pool class - Best practices? (node-postgres issue #3016)'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
---

# AWS Lambda + RDS Proxy and Pool Class - Best Practices

**Source:** node-postgres GitHub repository, issue #3016

## Issue Summary

This discussion addresses optimal strategies for using PostgreSQL connection pools (`Pool`) with AWS Lambda and RDS Proxy.

## Core Question

Should a Lambda function create a new pool for each invocation (establishing connections, running concurrent queries, closing in a `finally` block), or should it maintain persistent pools across invocations?

## Expert Guidance

**Sehrope Sarkuni** (contributor) provided the key clarification:

> "Lambda and similar environments are special as the freeze/thaw cycle doesn't allow for long term holding of TCP sockets. It's best to treat each request as stateless."

The recommendation: use `pool.query(...)` directly for independent operations, or run queries sequentially when order matters.

## Technical Recommendations

### 1. Per-Request Approach (Simple)

For straightforward cases, use `pool.query()` directly without manually acquiring a client:

```javascript
// Simple - pool manages connection acquisition and release automatically
const result = await pool.query('SELECT * FROM users WHERE id = $1', [userId]);
```

### 2. Concurrent Queries

Use multiple connections from the pool via separate `pool.query()` calls rather than sequential queries on a single client:

```javascript
// Concurrent - runs in parallel, uses multiple pool connections
const [users, posts] = await Promise.all([
  pool.query('SELECT * FROM users'),
  pool.query('SELECT * FROM posts'),
]);
```

### 3. Pool Sizing for Lambda

Limit pool size to 1 per Lambda instance to prevent resource monopolization:

```javascript
const pool = new Pool({
  connectionString: process.env.DATABASE_URL,
  max: 1, // Single connection per Lambda instance
  min: 0,
  idleTimeoutMillis: 120000,
  connectionTimeoutMillis: 10000,
});
```

Combined with RDS Proxy, the proxy handles the real pooling while each Lambda instance holds at most 1 connection.

### 4. Connection Release Pattern

When manually acquiring a client, always release in a `finally` block. Pass `true` to destroy the connection on error:

```javascript
const client = await pool.connect();
try {
  const result = await client.query('SELECT * FROM users');
  return result.rows;
} finally {
  client.release(true); // true = destroy connection on error states
}
```

### 5. Connection Limitations

A single PostgreSQL connection executes queries **serially**. For true concurrency, separate connections are required. Using `pool.query()` automatically handles this by borrowing separate connections from the pool.

## The Freeze/Thaw Problem

Lambda containers are "frozen" between invocations and "thawed" (reused) for subsequent calls. TCP sockets in a frozen container may become stale:

- Connections to RDS Proxy are more resilient because the proxy maintains the real connections on the database side
- Even with connection reuse, validate connection health on thaw
- Setting `idleTimeoutMillis` handles stale connections from Lambda instances that were cold for a long time

## Recommended Pattern for Lambda + node-postgres + RDS Proxy

```javascript
'use strict';

const { Pool } = require('pg');

// Initialize outside handler — reused across warm invocations
let pool;

function getPool() {
  if (!pool) {
    pool = new Pool({
      host: process.env.RDS_PROXY_HOST, // RDS Proxy endpoint, not RDS directly
      database: process.env.DB_NAME,
      user: process.env.DB_USER,
      password: process.env.DB_PASSWORD, // Or IAM token
      port: 5432,
      max: 1,
      idleTimeoutMillis: 120000,
      connectionTimeoutMillis: 10000,
      ssl: { rejectUnauthorized: false }, // For RDS SSL
    });
  }
  return pool;
}

module.exports.handler = async (event, context) => {
  context.callbackWaitsForEmptyEventLoop = false;

  const pool = getPool();

  try {
    const result = await pool.query('SELECT * FROM users WHERE id = $1', [
      event.userId,
    ]);
    return {
      statusCode: 200,
      body: JSON.stringify(result.rows),
    };
  } catch (error) {
    console.error('Database error:', error);
    throw error;
  }
};
```

## Important Note on API Design

The library maintainer acknowledged that supporting multiple concurrent queries on a single client is "a heinous API decision from 12 years ago, retained for legacy compatibility." This behavior should not be relied upon. Always use `pool.query()` for concurrent operations.

## Transaction Considerations

Multiple DML operations within a transaction **cannot** occur concurrently on a single connection — PostgreSQL itself enforces single-command-per-connection execution within a transaction. For transactions, acquire a single client and run statements sequentially:

```javascript
const client = await pool.connect();
try {
  await client.query('BEGIN');
  await client.query('INSERT INTO orders ...', [...]);
  await client.query('UPDATE inventory ...', [...]);
  await client.query('COMMIT');
} catch (e) {
  await client.query('ROLLBACK');
  throw e;
} finally {
  client.release();
}
```
