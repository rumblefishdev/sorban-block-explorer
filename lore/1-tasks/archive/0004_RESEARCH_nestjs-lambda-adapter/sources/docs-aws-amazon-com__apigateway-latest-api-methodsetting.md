---
url: 'https://docs.aws.amazon.com/apigateway/latest/api/API_MethodSetting.html'
title: 'MethodSetting - Amazon API Gateway API Reference'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
---

# MethodSetting - Amazon API Gateway

Specifies the method setting properties. Used to configure per-method (per-endpoint) cache TTL overrides and other method-level settings.

## Contents

### cacheDataEncrypted

Specifies whether the cached responses are encrypted.

- **Type:** Boolean
- **Required:** No

### cacheTtlInSeconds

Specifies the time to live (TTL), in seconds, for cached responses. The higher the TTL, the longer the response will be cached.

- **Type:** Integer
- **Required:** No

### cachingEnabled

Specifies whether responses should be cached and returned for requests. A cache cluster must be enabled on the stage for responses to be cached.

- **Type:** Boolean
- **Required:** No

### dataTraceEnabled

Specifies whether data trace logging is enabled for this method, which affects the log entries pushed to Amazon CloudWatch Logs. This can be useful to troubleshoot APIs, but can result in logging sensitive data. We recommend that you don't enable this option for production APIs.

- **Type:** Boolean
- **Required:** No

### loggingLevel

Specifies the logging level for this method, which affects the log entries pushed to Amazon CloudWatch Logs. Valid values are `OFF`, `ERROR`, and `INFO`. Choose `ERROR` to write only error-level entries to CloudWatch Logs, or choose `INFO` to include all `ERROR` events as well as extra informational events.

- **Type:** String
- **Required:** No

### metricsEnabled

Specifies whether Amazon CloudWatch metrics are enabled for this method.

- **Type:** Boolean
- **Required:** No

### requireAuthorizationForCacheControl

Specifies whether authorization is required for a cache invalidation request.

- **Type:** Boolean
- **Required:** No

### throttlingBurstLimit

Specifies the throttling burst limit.

- **Type:** Integer
- **Required:** No

### throttlingRateLimit

Specifies the throttling rate limit.

- **Type:** Double
- **Required:** No

### unauthorizedCacheControlHeaderStrategy

Specifies how to handle unauthorized requests for cache invalidation.

- **Type:** String
- **Valid Values:** `FAIL_WITH_403 | SUCCEED_WITH_RESPONSE_HEADER | SUCCEED_WITHOUT_RESPONSE_HEADER`
- **Required:** No

## Usage in CloudFormation (AWS::ApiGateway::Stage MethodSettings)

MethodSetting is used in the `MethodSettings` array of a Stage resource to configure per-endpoint cache TTL:

```yaml
ApiStage:
  Type: 'AWS::ApiGateway::Stage'
  Properties:
    StageName: Prod
    RestApiId: !Ref Api
    DeploymentId: !Ref ApiDeployment
    CacheClusterEnabled: true
    CacheClusterSize: '0.5'
    MethodSettings:
      - ResourcePath: '/*'
        HttpMethod: '*'
        CachingEnabled: true
        CacheTtlInSeconds: 300
      - ResourcePath: '/ledgers'
        HttpMethod: 'GET'
        CachingEnabled: true
        CacheTtlInSeconds: 3600
      - ResourcePath: '/transactions'
        HttpMethod: 'GET'
        CachingEnabled: true
        CacheTtlInSeconds: 60
```

The `ResourcePath` uses `~1` encoding for `/` in path segments (e.g., `/~1pets/GET` targets the GET method on the `/pets` resource).
