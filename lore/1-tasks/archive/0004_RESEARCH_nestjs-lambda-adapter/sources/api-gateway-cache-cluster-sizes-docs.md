---
url: 'https://docs.aws.amazon.com/apigateway/latest/developerguide/api-gateway-caching.html'
title: 'Cache settings for REST APIs in API Gateway'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 2
images:
  - original_url: 'https://docs.aws.amazon.com/images/apigateway/latest/developerguide/images/api-caching-stage-flow.png'
    local_path: 'images/api-gateway-cache-cluster-sizes-docs/img_1.png'
    alt: "AWS API Gateway console 'Additional settings' panel showing two toggles: 'Provision API cache' (enabled, described as provisioning API caching capabilities for the stage) and 'Default method-level caching' (enabled, described as activating method-level caching for all GET methods)."
  - original_url: 'https://docs.aws.amazon.com/images/apigateway/latest/developerguide/images/api-caching-including-parameter-as-cache-key-new-console.png'
    local_path: 'images/api-gateway-cache-cluster-sizes-docs/img_2.png'
    alt: "AWS API Gateway console 'Edit method request' form showing a 'URL query string parameters' table with columns for Name, Required, and Caching. Two parameters are listed: 'page' and 'type'. The 'type' parameter has the Caching checkbox checked, while 'page' does not."
---

# Cache settings for REST APIs in API Gateway

You can enable API caching in API Gateway to cache your endpoint's responses. With caching, you can reduce the number of calls made to your endpoint and also improve the latency of requests to your API.

When you enable caching for a stage, API Gateway caches responses from your endpoint for a specified time-to-live (TTL) period, in seconds. API Gateway then responds to the request by looking up the endpoint response from the cache instead of making a request to your endpoint. The default TTL value for API caching is 300 seconds. The maximum TTL value is 3600 seconds. TTL=0 means caching is disabled.

> **Note**
>
> Caching is best-effort. You can use the `CacheHitCount` and `CacheMissCount` metrics in Amazon CloudWatch to monitor requests that API Gateway serves from the API cache.

The maximum size of a response that can be cached is 1048576 bytes. Cache data encryption may increase the size of the response when it is being cached.

> **Important**
>
> When you enable caching for a stage, only `GET` methods have caching enabled by default. This helps to ensure the safety and availability of your API. You can enable caching for other methods by overriding method settings.

> **Important**
>
> Caching is charged by the hour based on the cache size that you select. Caching is not eligible for the AWS Free Tier. For more information, see [API Gateway Pricing](https://aws.amazon.com/api-gateway/pricing/).

## Cache cluster sizes

The valid cache cluster sizes (in GB) are:

**`0.5 | 1.6 | 6.1 | 13.5 | 28.4 | 58.2 | 118 | 237`**

Source: [`cacheClusterSize` parameter in the API Gateway API Reference](https://docs.aws.amazon.com/apigateway/latest/api/API_CreateStage.html#apigw-CreateStage-request-cacheClusterSize)

> The stage's cache capacity in GB. For more information about choosing a cache size, see Enabling API caching to enhance responsiveness.
>
> Type: String
>
> Valid Values: `0.5 | 1.6 | 6.1 | 13.5 | 28.4 | 58.2 | 118 | 237`
>
> Required: No

## Enable Amazon API Gateway caching

In API Gateway, you can enable caching for a specific stage.

When you enable caching, you must choose a cache capacity. In general, a larger capacity gives a better performance, but also costs more.

API Gateway enables caching by creating a dedicated cache instance. This process can take up to 4 minutes.

API Gateway changes caching capacity by removing the existing cache instance and creating a new one with a modified capacity. All existing cached data is deleted.

> **Note**
>
> The cache capacity affects the CPU, memory, and network bandwidth of the cache instance. As a result, the cache capacity can affect the performance of your cache.

API Gateway recommends that you run a 10-minute load test to verify that your cache capacity is appropriate for your workload. Ensure that traffic during the load test mirrors production traffic. For example, include ramp up, constant traffic, and traffic spikes. The load test should include responses that can be served from the cache, as well as unique responses that add items to the cache. Monitor the latency, 4xx, 5xx, cache hit, and cache miss metrics during the load test. Adjust your cache capacity as needed based on these metrics.

### AWS Management Console

In the API Gateway console, you configure caching on the **Stages** page. You provision the stage cache and specify a default method-level cache setting. If you turn on the default method-level cache, method-level caching is turned on for all `GET` methods on your stage, unless that method has a method override.

**To configure API caching for a given stage:**

1. Sign in to the API Gateway console at [https://console.aws.amazon.com/apigateway](https://console.aws.amazon.com/apigateway).
2. Choose **Stages**.
3. In the **Stages** list for the API, choose the stage.
4. In the **Stage details** section, choose **Edit**.
5. Under **Additional settings**, for **Cache settings**, turn on **Provision API cache**.
6. To activate caching for your stage, turn on **Default method-level caching**.

   ![AWS API Gateway console 'Additional settings' panel showing two toggles: 'Provision API cache' (enabled, described as provisioning API caching capabilities for the stage) and 'Default method-level caching' (enabled, described as activating method-level caching for all GET methods).](images/api-gateway-cache-cluster-sizes-docs/img_1.png)

7. Choose **Save changes**.

### AWS CLI

The following `update-stage` command updates a stage to provision a cache and turns on method-level caching for all `GET` methods on your stage:

```bash
aws apigateway update-stage \
    --rest-api-id a1b2c3 \
    --stage-name 'prod' \
    --patch-operations file://patch.json
```

The contents of `patch.json`:

```json
[
  {
    "op": "replace",
    "path": "/cacheClusterEnabled",
    "value": "true"
  },
  {
    "op": "replace",
    "path": "/cacheClusterSize",
    "value": "0.5"
  },
  {
    "op": "replace",
    "path": "/*/*/caching/enabled",
    "value": "true"
  }
]
```

> **Note**
>
> Creating or deleting a cache takes about 4 minutes for API Gateway to complete.

## Override API Gateway stage-level caching for method-level caching

You can override stage-level cache settings by turning on or turning off caching for a specific method. You can also modify the TTL period or turn encryption on or off for cached responses.

### AWS CLI

The following `update-stage` command turns off the cache only for the `GET /pets` method:

```bash
aws apigateway update-stage \
    --rest-api-id a1b2c3 \
    --stage-name 'prod' \
    --patch-operations file://patch.json
```

The contents of `patch.json`:

```json
[
  {
    "op": "replace",
    "path": "/~1pets/GET/caching/enabled",
    "value": "false"
  }
]
```

## Use method or integration parameters as cache keys

You can use a method or integration parameter as cache keys to index cached responses. This includes custom headers, URL paths, or query strings. When you have a cache key, API Gateway caches the responses from each key value separately, including when the cache key isn't present.

> **Note**
>
> Cache keys are required when setting up caching on a resource.

For example, suppose you have a request in the following format:

```
GET /users?type=... HTTP/1.1
host: example.com
```

If you include the `type` parameter as part of the cache key, the responses from `GET /users?type=admin` are cached separately from those from `GET /users?type=regular`.

### AWS Management Console

To include a method or integration request parameter as part of a cache key in the API Gateway console, select **Caching** after you add the parameter.

![AWS API Gateway console 'Edit method request' form showing a 'URL query string parameters' table with columns for Name, Required, and Caching. Two parameters are listed: 'page' and 'type'. The 'type' parameter has the Caching checkbox checked, while 'page' does not.](images/api-gateway-cache-cluster-sizes-docs/img_2.png)

### AWS CLI

The following `put-method` command creates a `GET` method and requires the `type` query string parameter:

```bash
aws apigateway put-method \
    --rest-api-id a1b2c3 \
    --resource-id aaa111 \
    --http-method GET \
    --authorization-type "NONE" \
    --request-parameters "method.request.querystring.type=true"
```

The following `put-integration` command creates an integration for the `GET` method and specifies that API Gateway caches the `type` method request parameter:

```bash
aws apigateway put-integration \
    --rest-api-id a1b2c3 \
    --resource-id aaa111 \
    --http-method GET \
    --type HTTP \
    --integration-http-method GET \
    --uri 'https://example.com' \
    --cache-key-parameters "method.request.querystring.type"
```

## Flush the API stage cache

When API caching is enabled, you can flush your API stage's cache to ensure that your API's clients get the most recent responses from your integration endpoints.

### AWS CLI

```bash
aws apigateway flush-stage-cache \
    --rest-api-id a1b2c3 \
    --stage-name prod
```

> **Note**
>
> After the cache is flushed, responses are serviced from the integration endpoint until the cache is built up again. During this period, the number of requests sent to the integration endpoint may increase.

## Invalidate an API Gateway cache entry

A client of your API can invalidate an existing cache entry and reload it from the integration endpoint for individual requests. The client must send a request that contains the `Cache-Control: max-age=0` header.

To grant permission for a client, attach a policy of the following format to an IAM execution role for the user.

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["execute-api:InvalidateCache"],
      "Resource": [
        "arn:aws:execute-api:us-east-1:111111111111:api-id/stage-name/GET/resource-path-specifier"
      ]
    }
  ]
}
```

When the policy is in place, caching is enabled and authorization is required. You can specify how API Gateway handles unauthorized requests:

- **Fail the request with 403 status code** (`FAIL_WITH_403`) — API Gateway returns a `403 Unauthorized` response.
- **Ignore cache control header; Add a warning in response header** (`SUCCEED_WITH_RESPONSE_HEADER`) — API Gateway processes the request and adds a warning header in the response.
- **Ignore cache control header** (`SUCCEED_WITHOUT_RESPONSE_HEADER`) — API Gateway processes the request and doesn't add a warning header.

## CloudFormation example of a stage with a cache

The following CloudFormation template creates an example API, provisions a `0.5` GB cache for the `Prod` stage, and turns on method-level caching for all `GET` methods.

```yaml
AWSTemplateFormatVersion: 2010-09-09
Resources:
  Api:
    Type: 'AWS::ApiGateway::RestApi'
    Properties:
      Name: cache-example
  PetsResource:
    Type: 'AWS::ApiGateway::Resource'
    Properties:
      RestApiId: !Ref Api
      ParentId: !GetAtt Api.RootResourceId
      PathPart: 'pets'
  PetsMethodGet:
    Type: 'AWS::ApiGateway::Method'
    Properties:
      RestApiId: !Ref Api
      ResourceId: !Ref PetsResource
      HttpMethod: GET
      ApiKeyRequired: true
      AuthorizationType: NONE
      Integration:
        Type: HTTP_PROXY
        IntegrationHttpMethod: GET
        Uri: http://petstore-demo-endpoint.execute-api.com/petstore/pets/
  ApiDeployment:
    Type: 'AWS::ApiGateway::Deployment'
    DependsOn:
      - PetsMethodGet
    Properties:
      RestApiId: !Ref Api
  ApiStage:
    Type: 'AWS::ApiGateway::Stage'
    Properties:
      StageName: Prod
      Description: Prod Stage with a cache
      RestApiId: !Ref Api
      DeploymentId: !Ref ApiDeployment
      CacheClusterEnabled: True
      CacheClusterSize: 0.5
      MethodSettings:
        - ResourcePath: /*
          HttpMethod: '*'
          CachingEnabled: True
```
