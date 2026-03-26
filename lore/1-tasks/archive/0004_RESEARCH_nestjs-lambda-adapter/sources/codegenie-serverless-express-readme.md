---
url: 'https://github.com/CodeGenieApp/serverless-express'
title: 'CodeGenieApp/serverless-express - GitHub README'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
---

# Serverless Express

> v5.0.0 Beta Released! Version 5.0.0 is now available in beta with Node.js 24 support, removal of deprecated APIs, and a fix for nested routes and custom domains.

Run REST APIs and other web applications using your existing [Node.js](https://nodejs.org/) application framework (Express, Koa, Hapi, Sails, etc.), on top of [AWS Lambda](https://aws.amazon.com/lambda/) and [Amazon API Gateway](https://aws.amazon.com/api-gateway/) or [Azure Function](https://docs.microsoft.com/en-us/azure/azure-functions/).

```bash
npm install @codegenie/serverless-express
```

> **Note on package history:** `aws-serverless-express` was deprecated in favor of `@vendia/serverless-express`, which was then rebranded to `@codegenie/serverless-express`. Brett Andrews, the original creator, continues maintaining the repository alongside the Code Genie team.

## Quick Start/Example

Check out the [basic starter example](https://github.com/CodeGenieApp/serverless-express/blob/mainline/examples/basic-starter-api-gateway-v1) that includes:

- Lambda function
- Express application
- Serverless Application Model (SAM)/CloudFormation template
- Helper scripts to configure, deploy, and manage your application

If you want to migrate an existing application to AWS Lambda, it's advised to get the minimal example up and running first, and then copy your application source in.

## AWS

### Minimal Lambda handler wrapper

The only AWS Lambda specific code you need to write is a simple handler. All other code you can write as you normally do.

```javascript
// lambda.js
const serverlessExpress = require('@codegenie/serverless-express');
const app = require('./app');
exports.handler = serverlessExpress({ app });
```

### Async setup Lambda handler

If your application needs to perform some common bootstrap tasks such as connecting to a database before the request is forwarded to the API:

```javascript
// lambda.js
require('source-map-support/register');
const serverlessExpress = require('@codegenie/serverless-express');
const app = require('./app');

let serverlessExpressInstance;

function asyncTask() {
  return new Promise((resolve) => {
    setTimeout(() => resolve('connected to database'), 1000);
  });
}

async function setup(event, context) {
  const asyncValue = await asyncTask();
  console.log(asyncValue);
  serverlessExpressInstance = serverlessExpress({ app });
  return serverlessExpressInstance(event, context);
}

function handler(event, context) {
  if (serverlessExpressInstance)
    return serverlessExpressInstance(event, context);

  return setup(event, context);
}

exports.handler = handler;
```

## Azure

### Async Azure Function v3/v4 handler wrapper

The only Azure Function specific code you need to write is a simple `index.js` and a `function.json`:

```javascript
// index.js
const serverlessExpress = require('@codegenie/serverless-express');
const app = require('./app');
const cachedServerlessExpress = serverlessExpress({ app });

module.exports = async function (context, req) {
  return cachedServerlessExpress(context, req);
};
```

```json
// function.json
{
  "bindings": [
    {
      "authLevel": "anonymous",
      "type": "httpTrigger",
      "direction": "in",
      "name": "req",
      "route": "{*segments}"
    },
    {
      "type": "http",
      "direction": "out",
      "name": "$return"
    }
  ]
}
```

The `"name": "$return"` out-binding parameter is important for Serverless Express to work.

## 4.x

Key improvements in version 4.x:

1. Improved API - Simpler for end-users to use and configure
2. Promise resolution mode by default with options for `"CONTEXT"` or `"CALLBACK"`
3. Additional event sources - API Gateway V1 (REST API), API Gateway V2 (HTTP API), ALB, Lambda@Edge, VPC Lattice
4. Custom event source support for unsupported event sources (see [DynamoDB Example](https://github.com/CodeGenieApp/serverless-express/blob/mainline/examples/custom-mapper-dynamodb))
5. Implementation uses mock Request/Response objects instead of running a server listening on a local socket
6. Automatic `isBase64Encoded` handling without specifying `binaryMimeTypes`
7. `respondWithErrors` makes it easier to debug during development
8. Node.js 12+ support
9. Improved support for custom domain names

See [UPGRADE.md](https://github.com/CodeGenieApp/serverless-express/blob/mainline/UPGRADE.md) to upgrade from `aws-serverless-express` and `@codegenie/serverless-express` 3.x.

## API

### binarySettings

Determine if the response should be base64 encoded before being returned to the event source, for example, when returning images or compressed files. By default, this is determined based on the `content-encoding` and `content-type` headers. For additional control, specify `binarySettings`:

```javascript
{
  binarySettings: {
    isBinary: ({ headers }) => true,
    contentTypes: ['image/*'],
    contentEncodings: []
  }
}
```

In SAM:

```yaml
ExpressApi:
  Type: AWS::Serverless::Api
  Properties:
    StageName: prod
    BinaryMediaTypes: ['image/*']
```

### resolutionMode (default: `'PROMISE'`)

Lambda supports three methods to end execution and return a result: context, callback, and promise. By default, serverless-express uses promise resolution, but you can specify `'CONTEXT'` or `'CALLBACK'`:

```javascript
serverlessExpress({
  app,
  resolutionMode: 'CALLBACK',
});
```

When set to `'CALLBACK'`, `context.callbackWaitsForEmptyEventLoop = false` is also configured.

### respondWithErrors (default: `process.env.NODE_ENV === 'development'`)

Set this to true to include the error stack trace in the event of an unhandled exception. By default, this is enabled when `NODE_ENV === 'development'` so that stack traces aren't returned in production.

## Advanced API

### eventSource

serverless-express natively supports API Gateway, ALB, Lambda@Edge and VPC Lattice (only V2 events). For other AWS Services, provide custom request/response mappings:

```javascript
function requestMapper({ event }) {
  // Your logic here...
  return {
    method,
    path,
    headers,
  };
}

function responseMapper({ statusCode, body, headers, isBase64Encoded }) {
  // Your logic here...
  return {
    statusCode,
    body,
    headers,
    isBase64Encoded,
  };
}

serverlessExpress({
  app,
  eventSource: {
    getRequest: requestMapper,
    getResponse: responseMapper,
  },
});
```

#### eventSourceRoutes

A single function can be configured to handle additional kinds of AWS events:

- SNS
- DynamoDB Streams
- SQS
- EventBridge Events

```yaml
# serverless.yml
functions:
  lambda-handler:
    handler: src/lambda.handler
    events:
      - http:
          path: /
          method: get
      - sns:
          topicName: my-topic
      - stream:
          type: dynamodb
          arn: arn:aws:dynamodb:us-east-1:012345678990:table/my-table/stream/2021-07-15T15:05:51.683
      - sqs:
          arn: arn:aws:sqs:us-east-1:012345678990:myQueue
      - eventBridge:
          pattern:
            source:
              - aws.cloudformation
```

```javascript
serverlessExpress({
  app,
  eventSourceRoutes: {
    AWS_SNS: '/sns',
    AWS_DYNAMODB: '/dynamodb',
    AWS_SQS: '/sqs',
    AWS_EVENTBRIDGE: '/eventbridge',
    AWS_KINESIS_DATA_STREAM: '/kinesis',
    AWS_S3: '/s3',
    AWS_STEP_FUNCTIONS: '/step-functions',
    AWS_SELF_MANAGED_KAFKA: '/self-managed-kafka',
  },
});
```

Events will `POST` to the configured routes.

For security, ensure the `Host` header matches:

- SNS: `sns.amazonaws.com`
- DynamoDB: `dynamodb.amazonaws.com`
- SQS: `sqs.amazonaws.com`
- EventBridge: `events.amazonaws.com`
- Kinesis Data Stream: `kinesis.amazonaws.com`

### logSettings

```javascript
{
  logSettings: {
    level: 'debug'; // default: 'error'
  }
}
```

### log

Provide a custom `log` object with `info`, `debug` and `error` methods:

```javascript
{
  log: {
    info (message, additional) {
      console.info(message, additional)
    },
    debug (message, additional) {
      console.debug(message, additional)
    },
    error (message, additional) {
      console.error(message, additional)
    }
  }
}
```

## Accessing the event and context objects

```javascript
const { getCurrentInvoke } = require('@codegenie/serverless-express');
app.get('/', (req, res) => {
  const { event, context } = getCurrentInvoke();
  res.json(event);
});
```

## Why run Express in a Serverless environment

- Only pay for what you use
- No infrastructure to manage
- Auto-scaling with zero configuration
- Usage Plans
- Caching
- Authorization
- Staging
- SDK Generation
- API Monitoring
- Request Validation
- Documentation

## Loadtesting

```bash
npx loadtest --rps 100 -k -n 1500 -c 50 https://xxxx.execute-api.us-east-1.amazonaws.com/prod/users
```

## Package History

On 11/30, the AWS Serverless Express library moved from AWS to Vendia and was rebranded to `@vendia/serverless-express`. The `aws-serverless-express` NPM package is deprecated in favor of `@vendia/serverless-express`, which was subsequently rebranded again as `@codegenie/serverless-express`.

Brett Andrews, the original creator, continues maintaining the repository. AWS and the SAM team remain involved administratively alongside Code Genie, Brett, and new maintainers.
