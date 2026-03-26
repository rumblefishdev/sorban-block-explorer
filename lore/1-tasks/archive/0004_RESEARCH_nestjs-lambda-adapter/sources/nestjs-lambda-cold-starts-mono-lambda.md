---
url: 'https://dev.to/aws-builders/aws-lambda-cold-starts-the-case-of-a-nestjs-mono-lambda-api-4j42'
title: 'AWS Lambda Cold Starts: The Case of a NestJS Mono-Lambda API'
author: 'Marko Djakovic'
date: '2022-07-08'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
---

# AWS Lambda Cold Starts: The Case of a NestJS Mono-Lambda API

**Posted by Marko Djakovic** for AWS Community Builders on Jul 8, 2022 (Edited Jun 20, 2025)

**Tags:** aws, serverless, nestjs, lambda

---

## Overview

This article explores AWS Lambda cold starts, specifically in the context of a mono-lambda (fat lambda) architecture using NestJS. The author demonstrates how consolidating multiple Lambda functions into a single function with multiple HTTP events can reduce cold start occurrences while maintaining detailed monitoring capabilities.

## What is a Cold Start?

Cold starts occur when Lambda functions execute for the first time after being idle. Even if the majority of requests are processed really fast, those that are impacted by the cold start will be noticeably slower.

## The Architecture Challenge

### Initial Approach (Function Per Endpoint)

The team initially structured their serverless configuration with separate Lambda functions per endpoint:

```yaml
functions:
  getSongs:
    handler: dist/lambda.handler
    events:
      - http:
          method: get
          path: songs
  getOneSong:
    handler: dist/lambda.handler
    events:
      - http:
          method: get
          path: songs/{id}
```

This approach caused performance degradation as each request frequently triggered new Lambda initializations.

### Solutions Considered

1. **Lambda Warming**: Using scheduled pings to keep functions warm (unreliable per AWS documentation)
2. **Provisioned Concurrency**: AWS's official recommendation, but costly and requires known usage patterns

### Final Solution (Mono-Lambda)

Instead, they consolidated to a single Lambda with multiple HTTP events:

```yaml
functions:
  api:
    handler: dist/lambda.handler
    events:
      - http:
          method: get
          path: songs
      - http:
          method: get
          path: songs/{id}
      - http:
          method: post
          path: songs
```

## Performance Testing Results

The testing involved approximately 10 requests per second for two minutes using Apache Bench.

### Key Metrics

- **Concurrent Executions**: 4 Lambda instances (4 cold starts for ~1000 requests)
- **Cold Start Impact**: Less than 0.5% of requests affected
- **Init Duration**: 1.0-1.1 seconds per cold start
- **Warm Request Performance**: Average 70ms response time
- Non-init Lambda execution times peaked under 400ms

## Conclusions

The author suggests that moderate-sized APIs deployed as mono-lambdas can perform adequately without becoming prohibitively slow. The architecture offers advantages for newer applications with uncertain usage patterns. As APIs grow, they can be refactored into microservices, and performance-critical applications may benefit from provisioned concurrency once usage patterns become predictable.

---

**Related Resource:** The article references [Deploy NestJS API to AWS Lambda with Serverless](https://marko.dj/posts/2022-04-20-deploy-nestjs-api-aws-lambda-serverless/) for deployment setup guidance.
