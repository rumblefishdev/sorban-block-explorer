---
url: 'https://elvisciotti.medium.com/aws-serverless-with-node-cache-data-for-a-certain-amount-of-time-ttl-2e8897e971cc'
title: 'AWS Serverless with Node: Cache Data for a Certain Amount of Time (TTL)'
author: 'Elvis Ciotti'
date: '2024-01-31'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
---

# AWS Serverless with Node: Cache Data for a Certain Amount of Time (TTL)

By Elvis Ciotti | 2 min read | Jan 31, 2024

## Overview

In AWS serverless environments, code executed outside the handler runs only once during cold start. For the next 15 minutes, subsequent invocations are "warm starts," meaning external code won't re-execute. This timeframe is ideal for database connections, constant initialization, and caching. See the [AWS documentation about execution environments](https://docs.aws.amazon.com/lambda/latest/operatorguide/execution-environments.html).

Global variables defined outside handlers persist during this period. You can test this by placing `let counter=0` outside the handler, incrementing it inside, and observing the value increase on subsequent invocations within the 15-minute window.

## Cache Data for the Whole Lambda Lifecycle (15 Minutes)

Simply define a cache object outside the handler:

```javascript
let cache = {};
```

Then use it inside the handler:

```javascript
/**
 * @param {import('aws-lambda').APIGatewayEvent} event
 */
export handler = async (event) => {
  const cacheKey = 'mykey';
  if (cache[cacheKey]) {
    console.log("return cached data");
    return cache[cacheKey];
  }
  // expensive operation e.g. network call
  console.log("calling the network to fetch data");
  const dataExpensiveToCalculate = await axios.get(...);
  cache[cacheKey] = dataExpensiveToCalculate;

  return;
}
```

The conditional logic typically belongs in a separate file or service, though this example is simplified for clarity.

## Cache Data for a Defined Amount of Time (e.g. 30 Seconds)

To expire cached data before the lambda lifecycle ends, manually track timestamps or use a library. The [node-cache](https://www.npmjs.com/package/node-cache) package handles this automatically.

Install with:

```bash
npm install node-cache --save
```

Use it like this:

```javascript
import NodeCache from 'node-cache';

const cache = new NodeCache();

/**
 * @param {import('aws-lambda').APIGatewayEvent} event
 */
export handler = async (event) => {
  const cacheKey = 'mykey';
  const cacheLifetimeSeconds = 30;

  if (cache.has(cacheKey)) {
    console.log("return cached data");
    return cache.get(cacheKey);
  }
  // expensive operation e.g. network call
  console.log("calling the network to fetch data");
  const networkData = await axios.get(...);
  cache.set(cacheKey, networkData, cacheLifetimeSeconds);

  return;
}
```

The `set` method requires a TTL (time to live) parameter in seconds. With a 30-second TTL, the handler returns cached data for 30 seconds before `has()` returns false and fetches fresh data.

## Key Points

- Code outside the handler runs once per cold start, not per invocation
- The Lambda execution environment typically stays alive for ~15 minutes of inactivity before being recycled
- Use a plain object `{}` for simple lifetime-of-container caching (no TTL)
- Use `node-cache` when you need sub-lifecycle TTL (e.g., refresh data every 30s, 5 minutes, etc.)
- `node-cache` TTL is in **seconds** (unlike `cache-manager` v5 which uses milliseconds)
- The `has()` + `get()` pattern avoids stale reads; alternatively use `get()` and check for `undefined`
