---
prefix: R
title: 'API Gateway Response Caching Strategy'
status: mature
spawned_from: '0004'
---

# R: API Gateway Response Caching Strategy

## Cache Architecture

**Important constraint:** CloudFront is reserved for static frontend/document delivery in the initial topology — API responses do NOT traverse CloudFront at launch. This means **API Gateway cache is the only HTTP-level caching layer** between the browser and Lambda. There is no CDN cache in front of the API, making API Gateway's built-in response cache critical for reducing Lambda invocations and database load.

API Gateway REST API supports **per-method cache TTL overrides** via `MethodSettings`. A cache cluster is provisioned at the stage level, then individual endpoints can enable/disable caching and set custom TTLs.

> Source: [api-gateway-cache-cluster-sizes-docs.md](../sources/api-gateway-cache-cluster-sizes-docs.md)
> Source: [docs-aws-amazon-com\_\_apigateway-latest-api-methodsetting.md](../sources/docs-aws-amazon-com__apigateway-latest-api-methodsetting.md)

## TTL Tiers for Block Explorer

### Tier 1: Immutable/Historical Data (long TTL)

These endpoints return data that never changes once confirmed:

| Endpoint Pattern                      | TTL                | Rationale                                     |
| ------------------------------------- | ------------------ | --------------------------------------------- |
| `GET /ledgers/{sequence}`             | **3600s** (1 hour) | Closed ledgers are immutable                  |
| `GET /transactions/{hash}`            | **3600s** (1 hour) | Confirmed transactions never change           |
| `GET /transactions/{hash}/operations` | **3600s**          | Operations within a transaction are immutable |
| `GET /contracts/{id}/interface`       | **3600s**          | Contract WASM interface doesn't change        |

### Tier 2: Semi-Stable Data (medium TTL)

Data that changes infrequently:

| Endpoint Pattern       | TTL              | Rationale                                     |
| ---------------------- | ---------------- | --------------------------------------------- |
| `GET /accounts/{id}`   | **60s**          | Balance changes with transactions             |
| `GET /tokens`          | **300s** (5 min) | Token list grows slowly                       |
| `GET /tokens/{id}`     | **60s**          | Token metadata rarely changes but supply does |
| `GET /contracts/{id}`  | **60s**          | Contract metadata + invocation counts update  |
| `GET /nfts`            | **300s**         | NFT listing changes infrequently              |
| `GET /liquidity-pools` | **60s**          | Pool data changes with trades                 |

### Tier 3: Mutable/Real-Time Data (short TTL)

Data that changes frequently and needs freshness:

| Endpoint Pattern           | TTL       | Rationale                                         |
| -------------------------- | --------- | ------------------------------------------------- |
| `GET /network`             | **5-15s** | Network stats, latest ledger, freshness indicator |
| `GET /ledgers` (list)      | **15s**   | New ledgers close every ~5s                       |
| `GET /transactions` (list) | **15s**   | New transactions arrive constantly                |
| `GET /search`              | **15s**   | Search results should reflect recent state        |

### Tier 4: No Cache

| Endpoint Pattern                   | TTL               | Rationale                                  |
| ---------------------------------- | ----------------- | ------------------------------------------ |
| Paginated lists with cursor params | **0 or disabled** | Cache key explosion from pagination params |

## CloudFormation Configuration

```yaml
ApiStage:
  Type: 'AWS::ApiGateway::Stage'
  Properties:
    StageName: prod
    RestApiId: !Ref Api
    DeploymentId: !Ref ApiDeployment
    CacheClusterEnabled: true
    CacheClusterSize: '0.5' # Start small, scale based on hit rate
    MethodSettings:
      # Default: cache disabled
      - ResourcePath: '/*'
        HttpMethod: '*'
        CachingEnabled: false
        CacheTtlInSeconds: 300
      # Tier 1: Immutable data
      - ResourcePath: '/~1ledgers~1{sequence}'
        HttpMethod: 'GET'
        CachingEnabled: true
        CacheTtlInSeconds: 3600
      - ResourcePath: '/~1transactions~1{hash}'
        HttpMethod: 'GET'
        CachingEnabled: true
        CacheTtlInSeconds: 3600
      # Tier 3: Real-time data
      - ResourcePath: '/~1network'
        HttpMethod: 'GET'
        CachingEnabled: true
        CacheTtlInSeconds: 15
```

> Source: [docs-aws-amazon-com\_\_apigateway-latest-api-methodsetting.md](../sources/docs-aws-amazon-com__apigateway-latest-api-methodsetting.md), CloudFormation example. Note: `~1` encodes `/` in resource paths.

## Cache Configuration Details

- **Default TTL:** 300s (5 min)
- **Maximum TTL:** 3600s (1 hour)
- **Cache cluster sizes:** 0.5GB, 1.6GB, 6.1GB, 13.5GB, 28.4GB, 58.2GB, 118GB, 237GB
- **Cache key:** By default, the full request URL. Can include headers, query strings, or path parameters via `CacheKeyParameters`.

> Source: [api-gateway-cache-cluster-sizes-docs.md](../sources/api-gateway-cache-cluster-sizes-docs.md), "Cache cluster sizes" and "Cache Configuration Details" sections

## Cache Invalidation

Clients can invalidate cache by sending `Cache-Control: max-age=0` header. This should be **restricted** via `unauthorizedCacheControlHeaderStrategy`:

- `FAIL_WITH_403` — reject invalidation requests without proper authorization
- `SUCCEED_WITH_RESPONSE_HEADER` — allow but add header indicating cache miss
- `SUCCEED_WITHOUT_RESPONSE_HEADER` — silently allow

For a public read-only API, use `FAIL_WITH_403` to prevent cache poisoning abuse.

> Source: [docs-aws-amazon-com\_\_apigateway-latest-api-methodsetting.md](../sources/docs-aws-amazon-com__apigateway-latest-api-methodsetting.md), `unauthorizedCacheControlHeaderStrategy` field

## Cache Key Strategy for Paginated Endpoints

For list endpoints with query params (page, limit, cursor, filters), the cache key must include these parameters. This can lead to cache key explosion. Two approaches:

1. **Don't cache paginated lists** — simpler, avoids key explosion
2. **Cache with query param keys** — only for first page / common filter combinations

**Recommendation:** Start with option 1 (no cache for paginated lists), add selective caching later based on traffic analysis.
