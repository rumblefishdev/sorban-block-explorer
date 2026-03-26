---
prefix: R
title: 'In-Memory Lambda Caching Pattern'
status: mature
spawned_from: '0004'
---

# R: In-Memory Lambda Caching Pattern

## How Lambda In-Memory Caching Works

Variables declared **outside the handler** (in module/global scope) persist across warm invocations. The execution environment typically stays alive for 15-40 minutes after the last invocation. On cold start, all cached data is lost and must be rebuilt.

> Source: [node-cache-ttl-lambda-pattern.md](../sources/node-cache-ttl-lambda-pattern.md), "execution environments... ~15 minutes"; [lambda-inmemory-cache-nodejs-patterns.md](../sources/lambda-inmemory-cache-nodejs-patterns.md), "Cold Start vs Warm Start" section

## Recommended Library: `node-cache`

For sub-lifecycle TTL (e.g., refresh data every 30-60 seconds), use `node-cache`:

```typescript
import NodeCache from 'node-cache';

// Instantiate outside handler — survives across warm invocations
const cache = new NodeCache();

// In a NestJS service:
async getNetworkStats(): Promise<NetworkStats> {
  const CACHE_KEY = 'network-stats';
  const TTL_SECONDS = 30;

  if (cache.has(CACHE_KEY)) {
    return cache.get<NetworkStats>(CACHE_KEY)!;
  }

  const stats = await this.db.select().from(networkStats).limit(1);
  cache.set(CACHE_KEY, stats, TTL_SECONDS);
  return stats;
}
```

> Source: [node-cache-ttl-lambda-pattern.md](../sources/node-cache-ttl-lambda-pattern.md) — `node-cache` TTL is in **seconds** (unlike `cache-manager` v5 which uses milliseconds)

### Why `node-cache` Over Alternatives

| Library                 | TTL Support   | Eviction     | Lambda Suitability               |
| ----------------------- | ------------- | ------------ | -------------------------------- |
| `node-cache`            | Yes (seconds) | Time-based   | Good — simple, lightweight       |
| `lru-cache`             | Yes           | Size + time  | Good — better for bounded memory |
| Plain object `{}`       | No (manual)   | None         | Good for full-lifecycle cache    |
| `@nestjs/cache-manager` | Yes           | Configurable | Overkill for Lambda in-memory    |

> Source: [lambda-inmemory-cache-nodejs-patterns.md](../sources/lambda-inmemory-cache-nodejs-patterns.md), library comparison

**For our use case:** `node-cache` is the simplest option with built-in TTL. `lru-cache` is a good alternative if memory bounds are a concern (it evicts least-recently-used entries when max size is reached).

## What to Cache In-Memory

Based on the architecture requirements, cache these data types with 30-60s TTLs:

### 1. Network Stats (30s TTL)

- Latest ledger sequence, chain stats, freshness indicator
- Queried on every page load
- 30s TTL = max 30s stale, acceptable for a status indicator

### 2. Contract Metadata (60s TTL)

- Contract interface, WASM metadata
- Rarely changes, frequently accessed
- 60s TTL reduces DB round-trips significantly

### 3. Token Reference Data (60s TTL)

- Token list metadata (name, symbol, icon)
- Changes very rarely once established
- 60s TTL with high cache hit rate

## NestJS Integration Pattern

Two approaches for integrating with NestJS on Lambda:

### Option A: Module-Level `node-cache` (Recommended)

Create a simple caching service using `node-cache` at module scope:

```typescript
import NodeCache from 'node-cache';

const nodeCache = new NodeCache({ stdTTL: 60 }); // default 60s TTL

@Injectable()
export class CacheService {
  get<T>(key: string): T | undefined {
    return nodeCache.get<T>(key);
  }

  set<T>(key: string, value: T, ttlSeconds?: number): void {
    nodeCache.set(key, value, ttlSeconds ?? 0); // 0 = use stdTTL
  }

  has(key: string): boolean {
    return nodeCache.has(key);
  }
}
```

The `nodeCache` instance persists across warm invocations because the entire NestJS app is cached via the `cachedServer` pattern.

> Source: [nestjs-serverless-docs.md](../sources/nestjs-serverless-docs.md), "Example integration" section — `server = server ?? (await bootstrap())` pattern caches the NestJS app globally

## Important Caveats

1. **Cache is per-Lambda-instance** — If 4 Lambda instances are running, each has its own independent cache. No shared state between instances.

> Source: [lambda-inmemory-cache-nodejs-patterns.md](../sources/lambda-inmemory-cache-nodejs-patterns.md), "Each Lambda run is independent — there's no built-in way to share memory across different executions or instances"

2. **Cache lost on cold start** — Acceptable for our use case. First request after cold start fetches from DB, subsequent requests use cache.

> Source: [lambda-inmemory-cache-nodejs-patterns.md](../sources/lambda-inmemory-cache-nodejs-patterns.md), "Cache survives only in warm containers"

3. **Memory limits** — Lambda has finite memory. Keep cached data small (network stats, metadata, reference data). Don't cache full query results for large datasets.

4. **Consistency** — With 30-60s TTLs, data can be up to 60s stale. This is acceptable for a block explorer where the freshness indicator already shows the highest indexed ledger sequence.

5. **Graceful degradation when ingestion is delayed** — The architecture requires the backend to serve already-indexed data even when upstream ingestion is stalled. In-memory caching naturally supports this: cached data continues to be served from memory regardless of ingestion status. The `/network` endpoint's freshness indicator (highest indexed ledger sequence) tells users how current the data is. No special Lambda configuration is needed — this is an application-level concern handled by always querying the owned database (which holds whatever has been indexed so far) rather than depending on live upstream data.

## Two-Tier Caching Strategy

The architecture benefits from **two layers**:

1. **API Gateway cache** — Shared across all clients, reduces Lambda invocations entirely
2. **Lambda in-memory cache** — Per-instance, reduces DB queries when API Gateway cache misses

This means: API Gateway cache (15-3600s TTL) → Lambda in-memory cache (30-60s TTL) → Database

The in-memory cache is most valuable for endpoints where API Gateway caching is disabled or has very short TTLs (e.g., `/network` with 15s API Gateway TTL still benefits from 30s in-memory TTL to reduce DB load during burst traffic).
