---
prefix: R
title: 'Lambda Adapter Selection for NestJS'
status: mature
spawned_from: '0004'
---

# R: Lambda Adapter Selection for NestJS

## Recommendation: `@codegenie/serverless-express`

### Package Lineage

The adapter has gone through three naming iterations, all maintained by the original creator Brett Andrews:

1. `aws-serverless-express` — original AWS package (deprecated)
2. `@vendia/serverless-express` — rebranded under Vendia
3. `@codegenie/serverless-express` — current canonical package

> Source: [codegenie-serverless-express-readme.md](../sources/codegenie-serverless-express-readme.md) — "Note on package history" section

### Why This Adapter

1. **Official NestJS recommendation** — NestJS docs explicitly use `@codegenie/serverless-express` in their serverless example integration ([nestjs-serverless-docs.md](../sources/nestjs-serverless-docs.md), line 176: `npm i @codegenie/serverless-express aws-lambda`)

2. **Broad event source support** — Supports API Gateway V1 (REST API), API Gateway V2 (HTTP API), ALB, Lambda@Edge, and VPC Lattice. This means switching API Gateway modes doesn't require adapter changes ([codegenie-serverless-express-readme.md](../sources/codegenie-serverless-express-readme.md), "4.x" section, point 3)

3. **No local socket overhead** — v4.x uses mock Request/Response objects instead of running a local server on a socket, reducing cold start overhead ([codegenie-serverless-express-readme.md](../sources/codegenie-serverless-express-readme.md), "4.x" section, point 5)

4. **v5.0.0 Beta with Node.js 24 support** — Active development continues ([codegenie-serverless-express-readme.md](../sources/codegenie-serverless-express-readme.md), header note)

5. **Async bootstrap pattern** — Supports caching the NestJS app instance across warm invocations via the standard `cachedServer` pattern ([codegenie-serverless-express-readme.md](../sources/codegenie-serverless-express-readme.md), "Async setup Lambda handler" section)

### Handler Pattern for Our Project

The recommended pattern from NestJS official docs, combining with our mono-lambda approach:

```typescript
import { NestFactory } from '@nestjs/core';
import serverlessExpress from '@codegenie/serverless-express';
import { Callback, Context, Handler } from 'aws-lambda';
import { AppModule } from './app.module';

let server: Handler;

async function bootstrap(): Promise<Handler> {
  const app = await NestFactory.create(AppModule);
  await app.init();
  const expressApp = app.getHttpAdapter().getInstance();
  return serverlessExpress({ app: expressApp });
}

export const handler: Handler = async (
  event: any,
  context: Context,
  callback: Callback
) => {
  server = server ?? (await bootstrap());
  return server(event, context, callback);
};
```

> Source: [nestjs-serverless-docs.md](../sources/nestjs-serverless-docs.md), "Example integration" section

### Resolution Mode

Default resolution mode is `'PROMISE'` which is ideal for modern async/await Lambda handlers. Alternative modes `'CONTEXT'` and `'CALLBACK'` exist but are not needed. When using `'CALLBACK'` mode, the adapter automatically sets `context.callbackWaitsForEmptyEventLoop = false`.

> Source: [codegenie-serverless-express-readme.md](../sources/codegenie-serverless-express-readme.md), "resolutionMode" section

### Mono-Lambda Architecture

All 9 NestJS modules run in a single Lambda function with a catch-all route (`ANY {proxy+}`). NestJS handles internal routing. This approach:

- Reduces cold start frequency to <0.5% of requests at moderate traffic (10 req/s)
- Achieves ~70ms warm response time
- Requires only 4 concurrent Lambda instances for 10 req/s

> Source: [nestjs-lambda-cold-starts-mono-lambda.md](../sources/nestjs-lambda-cold-starts-mono-lambda.md), "Performance Testing Results" section

### Alternatives Considered

**`@nestjs/platform-express` alone** — This is not a standalone Lambda adapter. It is the Express HTTP platform that NestJS uses internally. `@codegenie/serverless-express` wraps the Express instance created by `@nestjs/platform-express` and translates API Gateway events into Express-compatible requests. They are complementary, not competing options.

> Source: [nestjs-serverless-docs.md](../sources/nestjs-serverless-docs.md), "Example integration" — `app.getHttpAdapter().getInstance()` returns the Express instance from `@nestjs/platform-express`, which is then passed to `serverlessExpress({ app: expressApp })`

No other viable alternatives exist. The entire NestJS ecosystem and community converge on `@codegenie/serverless-express`. No competing adapter has comparable maintenance, compatibility, or community adoption.

### Bundling

NestJS docs recommend webpack bundling with `libraryTarget: 'commonjs2'` for Lambda, which reduces bootstrap from 197.4ms to 81.5ms for a simple app ([nestjs-serverless-docs.md](../sources/nestjs-serverless-docs.md), benchmark tables). However, **esbuild** produces smaller bundles (75-90% reduction) with 40-90% cold start improvement over unbundled deployments ([lambda-bundle-size-esbuild-optimization.md](../sources/lambda-bundle-size-esbuild-optimization.md)).
