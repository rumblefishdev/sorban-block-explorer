---
prefix: R
title: 'API Gateway Mode: REST API vs HTTP API'
status: mature
spawned_from: '0004'
---

# R: API Gateway Mode — REST API vs HTTP API

## Recommendation: REST API

### Decision Matrix

| Requirement               | REST API                       | HTTP API | Verdict                |
| ------------------------- | ------------------------------ | -------- | ---------------------- |
| **Response caching**      | Yes (built-in, per-method TTL) | **No**   | REST API required      |
| **AWS WAF**               | Yes                            | **No**   | REST API required      |
| **Request validation**    | Yes                            | No       | REST API preferred     |
| **Per-client throttling** | Yes                            | No       | REST API preferred     |
| **API keys**              | Yes                            | No       | Not needed (anonymous) |

> Source: [docs-aws-amazon-com\_\_apigateway-latest-developerguide-http-api-vs-rest.md](../sources/docs-aws-amazon-com__apigateway-latest-developerguide-http-api-vs-rest.md), full feature comparison tables

### Why REST API Wins Despite Higher Cost

1. **Caching is REST API-only** — HTTP API has **no caching feature at all**. Our architecture requires response caching with different TTLs per endpoint. This is a hard requirement.

> Source: [docs-aws-amazon-com\_\_apigateway-latest-developerguide-http-api-vs-rest.md](../sources/docs-aws-amazon-com__apigateway-latest-developerguide-http-api-vs-rest.md), "Development" table — Caching: REST=Yes, HTTP=No

2. **WAF is REST API-only** — The architecture specifies WAF attachment for abuse protection. HTTP API does not support WAF natively.

> Source: [docs-aws-amazon-com\_\_apigateway-latest-developerguide-http-api-vs-rest.md](../sources/docs-aws-amazon-com__apigateway-latest-developerguide-http-api-vs-rest.md), "Security" table — AWS WAF: REST=Yes, HTTP=No

3. **Per-client rate limiting** — REST API supports per-client throttling which is useful for abuse prevention alongside WAF.

> Source: [docs-aws-amazon-com\_\_apigateway-latest-developerguide-http-api-vs-rest.md](../sources/docs-aws-amazon-com__apigateway-latest-developerguide-http-api-vs-rest.md), "API management" table

### Cost Consideration

HTTP API is cheaper per-request, but with response caching enabled on REST API, many requests never reach Lambda — reducing both Lambda invocation costs and effective API Gateway costs. The caching savings offset the REST API price premium.

### Additional REST API Benefits

- **X-Ray tracing** — useful for debugging latency issues
- **Execution logs** — detailed CloudWatch logging
- **Canary deployments** — safer rollouts
- **Custom gateway responses** — better error messages

> Source: [docs-aws-amazon-com\_\_apigateway-latest-developerguide-http-api-vs-rest.md](../sources/docs-aws-amazon-com__apigateway-latest-developerguide-http-api-vs-rest.md), "Monitoring" and "Development" tables

### WAF Evaluation Order

WAF evaluates **before all other access controls** (resource policies, IAM, Lambda authorizers). This means malicious requests are blocked at the edge before consuming any Lambda resources.

> Source: [docs-aws-amazon-com\_\_apigateway-latest-developerguide-apigateway-control-access-aws-waf.md](../sources/docs-aws-amazon-com__apigateway-latest-developerguide-apigateway-control-access-aws-waf.md)
