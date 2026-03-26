---
url: 'https://www.jeremydaly.com/reuse-database-connections-aws-lambda/'
title: 'How To: Reuse Database Connections in AWS Lambda'
author: 'Jeremy Daly'
date: '2017-09-29'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
---

# How To: Reuse Database Connections in AWS Lambda

**Author:** Jeremy Daly
**Published:** September 29, 2017

## Overview

AWS Lambda enables developers to maintain and reuse database connections between function invocations by leveraging the container freezing mechanism. This approach significantly reduces connection overhead compared to establishing fresh connections for each execution.

## The Connection Problem

Creating new database connections is computationally expensive. Setting up a new database connection typically takes more than 200ms. For API Gateway-triggered Lambda functions, this overhead becomes a serious performance problem.

## The Solution: Container Reuse

Lambda containers persist between invocations. Variables declared outside the handler function remain in memory and can be reused. Depending on the volume of executions, the container is almost always reused.

### Basic Implementation Pattern (Counter Example)

To demonstrate state persistence across Lambda invocations:

```javascript
'use strict';

let counter = 0;

module.exports.handler = (event, context, callback) => {
  counter++;
  console.log(counter);
  callback(null, { count: counter });
};
```

If the container is reused, the counter keeps incrementing. A value greater than 1 on the first call indicates container reuse.

### Database Connection Pattern

```javascript
'use strict';

const mysql = require('mysql');

if (typeof client === 'undefined') {
  var client = mysql.createConnection({
    // your connection info
  });

  client.connect();
}

module.exports.handler = (event, context, callback) => {
  context.callbackWaitsForEmptyEventLoop = false;

  client.query('SELECT * FROM `books`', function (error, results) {
    callback(null, results);
  });
};
```

### PostgreSQL Version (node-postgres)

```javascript
'use strict';

const { Pool } = require('pg');

let pool;

if (!pool) {
  pool = new Pool({
    connectionString: process.env.DATABASE_URL,
    max: 1,
    idleTimeoutMillis: 120000,
    connectionTimeoutMillis: 10000,
  });
}

module.exports.handler = async (event, context) => {
  context.callbackWaitsForEmptyEventLoop = false;

  const client = await pool.connect();
  try {
    const result = await client.query('SELECT * FROM books');
    return result.rows;
  } finally {
    client.release(true); // Pass true to discard connection on error
  }
};
```

## Critical Configuration: `callbackWaitsForEmptyEventLoop`

The `context.callbackWaitsForEmptyEventLoop` property **must** be set to `false`. This setting allows Lambda to return immediately after the callback executes, rather than waiting for the event loop to drain — which would prevent the connection from remaining open between invocations.

Without this setting, Lambda will wait for all event loop items (including open DB connections) to close before returning, defeating the purpose of connection reuse.

## Recommended Pool Configuration for Lambda

```javascript
const pool = new Pool({
  max: 1, // Single connection per Lambda instance
  min: 0, // No minimum maintained connections
  idleTimeoutMillis: 120000, // 2 minutes idle before release
  connectionTimeoutMillis: 10000, // 10 seconds to acquire connection
});
```

- **max: 1** — Each Lambda instance has its own connection; pool size 1 prevents resource monopolization
- **min: 0** — Don't hold connections when idle
- **idleTimeoutMillis: 120000** — Longer than typical RDS idle timeout to reuse warm connections
- **connectionTimeoutMillis: 10000** — Fail fast if DB is unreachable

## Advanced Patterns: Module Closures

Module references are also frozen between invocations. This enables encapsulating database logic in separate modules using closures:

```javascript
// db.js
'use strict';

const { Pool } = require('pg');

let pool;

module.exports = {
  getPool: () => {
    if (!pool) {
      pool = new Pool({
        connectionString: process.env.DATABASE_URL,
        max: 1,
        idleTimeoutMillis: 120000,
      });
    }
    return pool;
  },
  query: (text, params) => module.exports.getPool().query(text, params),
};
```

```javascript
// handler.js
const db = require('./db');

module.exports.handler = async (event, context) => {
  context.callbackWaitsForEmptyEventLoop = false;
  return await db.query('SELECT * FROM books');
};
```

## Performance Results

As of 2018 follow-up measurements, "warm" function connections to RDS instances in the same VPC averaged 4–20ms. New connections still add 200ms+ overhead, making reuse critical for performance-sensitive APIs.

## Related: serverless-mysql NPM Module

The author later published `serverless-mysql`, an NPM module that automates connection management patterns for serverless environments, handling connection pooling, reuse, and cleanup automatically.

## Related Resources

- [How To: Manage RDS Connections from AWS Lambda Serverless Functions](https://www.jeremydaly.com/manage-rds-connections-aws-lambda/)
