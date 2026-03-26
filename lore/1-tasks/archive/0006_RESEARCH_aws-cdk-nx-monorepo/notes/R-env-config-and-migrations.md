---
title: 'Environment configuration, secret handling, and schema migration in CDK'
type: research
status: developing
spawned_from: null
spawns: []
tags: [cdk, config, secrets, migrations, environments]
links:
  - https://docs.aws.amazon.com/cdk/v2/guide/environments.html
  - https://docs.aws.amazon.com/cdk/v2/guide/context.html
history:
  - date: 2026-03-26
    status: developing
    who: stkrolikiewicz
    note: 'Config and migration patterns based on architecture docs and ADR-0001'
---

# Environment configuration, secret handling, and schema migration in CDK

## Environment Configuration Approach

### Recommendation: TypeScript config module

Not CDK context (`cdk.json`), not environment variables — a typed TypeScript config module at `infra/aws-cdk/lib/config/`.

```typescript
// infra/aws-cdk/lib/config/types.ts
export interface EnvironmentConfig {
  readonly envName: 'staging' | 'production';
  readonly awsAccount: string;
  readonly awsRegion: string;

  // Network
  readonly vpcCidr: string;
  readonly availabilityZone: string;

  // RDS
  readonly dbInstanceClass: string;
  readonly dbMultiAz: boolean;
  readonly dbDeletionProtection: boolean;
  readonly dbBackupRetentionDays: number;

  // S3
  readonly ledgerDataRetentionDays: number;
  readonly kmsEncryption: boolean;

  // Lambda
  readonly apiLambdaMemory: number;
  readonly apiLambdaProvisionedConcurrency: number;
  readonly processorLambdaMemory: number;

  // API Gateway
  readonly apiThrottleRateLimit: number;
  readonly apiThrottleBurstLimit: number;
  readonly apiCacheClusterSize: string;

  // CloudFront
  readonly frontendPasswordProtected: boolean;

  // Monitoring
  readonly alarmActionsArn: string; // SNS topic ARN for paging
  readonly galexieLagThresholdSeconds: number;
  readonly rdsMaxCpuPercent: number;
  readonly api5xxRatePercent: number;

  // Secrets (references only, not values)
  readonly dbSecretArn: string;
  readonly stagingPasswordSecretArn?: string;

  // Domain
  readonly domainName: string;
  readonly hostedZoneId: string;
}
```

```typescript
// infra/aws-cdk/lib/config/staging.ts
import { EnvironmentConfig } from './types';

export const stagingConfig: EnvironmentConfig = {
  envName: 'staging',
  awsAccount: process.env.CDK_DEFAULT_ACCOUNT!,
  awsRegion: 'us-east-1',
  vpcCidr: '10.0.0.0/16',
  availabilityZone: 'us-east-1a',

  dbInstanceClass: 'db.t3.medium',
  dbMultiAz: false,
  dbDeletionProtection: false,
  dbBackupRetentionDays: 7,

  ledgerDataRetentionDays: 7,
  kmsEncryption: false, // SSE-S3 for staging

  apiLambdaMemory: 512,
  apiLambdaProvisionedConcurrency: 0,
  processorLambdaMemory: 256,

  apiThrottleRateLimit: 100,
  apiThrottleBurstLimit: 50,
  apiCacheClusterSize: '0.5',

  frontendPasswordProtected: true,

  galexieLagThresholdSeconds: 60,
  rdsMaxCpuPercent: 70,
  api5xxRatePercent: 0.5,

  alarmActionsArn: '', // email/Slack, non-paging
  dbSecretArn: '', // set via CDK context or env var
  stagingPasswordSecretArn: '', // set via CDK context or env var

  domainName: 'staging.explorer.example.com',
  hostedZoneId: '',
};
```

```typescript
// infra/aws-cdk/lib/config/production.ts
import { EnvironmentConfig } from './types';

export const productionConfig: EnvironmentConfig = {
  envName: 'production',
  awsAccount: process.env.CDK_DEFAULT_ACCOUNT!,
  awsRegion: 'us-east-1',
  vpcCidr: '10.1.0.0/16',
  availabilityZone: 'us-east-1a',

  dbInstanceClass: 'db.r6g.large',
  dbMultiAz: false, // Single-AZ at launch, toggle when SLA > 99.9%
  dbDeletionProtection: true,
  dbBackupRetentionDays: 30,

  ledgerDataRetentionDays: 30,
  kmsEncryption: true, // KMS-backed for production per architecture docs

  apiLambdaMemory: 1024,
  apiLambdaProvisionedConcurrency: 5,
  processorLambdaMemory: 512,

  apiThrottleRateLimit: 1000,
  apiThrottleBurstLimit: 500,
  apiCacheClusterSize: '1.6',

  frontendPasswordProtected: false,

  galexieLagThresholdSeconds: 60,
  rdsMaxCpuPercent: 70,
  api5xxRatePercent: 0.5,

  alarmActionsArn: '', // SNS topic for PagerDuty/paging
  dbSecretArn: '', // set per deployment

  domainName: 'explorer.example.com',
  hostedZoneId: '',
};
```

### Why TypeScript module, not CDK context

| Approach                 | Pros                                                                  | Cons                                                |
| ------------------------ | --------------------------------------------------------------------- | --------------------------------------------------- |
| **TypeScript module**    | Type-safe, IDE autocomplete, can validate at compile time, composable | Must import in CDK code                             |
| CDK context (`cdk.json`) | Built-in, can override via CLI `--context`                            | No type safety, flat key-value, hard to validate    |
| Environment variables    | 12-factor app pattern, CI-friendly                                    | No structure, no type safety, proliferates env vars |

TypeScript module is the clear winner for a project with 20+ config values and type safety matters.

### Account ID Handling (Open-Source Redeployability)

Per ADR-0001, no hard-coded account IDs:

```typescript
// infra/aws-cdk/bin/app.ts
const envName = process.env.CDK_ENV || 'staging';
const config = envName === 'production' ? productionConfig : stagingConfig;

// Account comes from environment, not hard-coded
const env = {
  account: config.awsAccount || process.env.CDK_DEFAULT_ACCOUNT,
  region: config.awsRegion,
};

const app = new cdk.App();
new NetworkStack(app, `Explorer-${envName}-Network`, { env, config });
// ...
```

Third parties override `awsAccount` in their config file or via `CDK_DEFAULT_ACCOUNT`.

## Secret Handling (ADR-0001)

Config files contain **secret references** (ARN/name), never values:

```typescript
// In stack code:
const dbSecret = secretsmanager.Secret.fromSecretCompleteArn(
  this,
  'DbSecret',
  config.dbSecretArn
);

// Lambda reads secret at runtime via IAM
processorLambda.addToRolePolicy(
  new iam.PolicyStatement({
    actions: ['secretsmanager:GetSecretValue'],
    resources: [config.dbSecretArn], // Scoped to specific ARN, not wildcard
  })
);
```

Staging web password:

```typescript
if (config.frontendPasswordProtected && config.stagingPasswordSecretArn) {
  // CloudFront Function reads password from Secrets Manager
  // or use Lambda@Edge with Secrets Manager access
}
```

## Schema Migration Strategy

### Recommendation: CDK Custom Resource with Lambda

A Lambda function that runs Drizzle Kit migrations, triggered as a CDK Custom Resource during deployment.

```typescript
import * as cr from 'aws-cdk-lib/custom-resources';
import * as crypto from 'crypto';
import * as fs from 'fs';

const migrationLambda = new lambda.Function(this, 'MigrationRunner', {
  runtime: lambda.Runtime.NODEJS_22_X,
  handler: 'index.handler',
  code: lambda.Code.fromAsset(
    path.join(__dirname, '../../../../apps/api/dist')
  ),
  vpc: props.vpc,
  securityGroups: [props.dbSecurityGroup],
  timeout: cdk.Duration.minutes(5),
});

// Grant DB + Secrets Manager access
migrationLambda.addToRolePolicy(/* RDS Proxy + Secrets Manager access */);

// Provider wraps the Lambda for Custom Resource invocation
const migrationProvider = new cr.Provider(this, 'MigrationProvider', {
  onEventHandler: migrationLambda,
});

// Hash of migration directory — triggers re-execution only when migrations change
const migrationsDir = path.join(__dirname, '../../../../drizzle/migrations');
const migrationHash = crypto
  .createHash('md5')
  .update(fs.readdirSync(migrationsDir).join(','))
  .digest('hex');

// Custom Resource — runs BEFORE dependent resources
const migration = new cdk.CustomResource(this, 'RunMigration', {
  serviceToken: migrationProvider.serviceToken,
  properties: {
    migrationHash, // change triggers re-execution
  },
});

// API Lambda depends on migration completing
apiLambda.node.addDependency(migration);
```

### Why Custom Resource Lambda, not CodeBuild

| Approach                   | Pros                                                                             | Cons                                                             |
| -------------------------- | -------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| **Custom Resource Lambda** | Runs within CDK deploy, automatic ordering via `addDependency`, no extra service | Cold start, 15-min timeout                                       |
| CodeBuild step             | No timeout limit, can run arbitrary scripts                                      | Separate service, harder to order with CDK resources, extra cost |
| Manual step                | Simple                                                                           | Not automated, blocks deployment pipeline                        |

Custom Resource Lambda is simplest for Drizzle Kit migrations (typically <30 seconds).

### Migration ordering in CDK

```
StorageStack (RDS, RDS Proxy)
    ↓ dependency
MigrationCustomResource (runs Drizzle Kit)
    ↓ dependency
ComputeStack (Lambdas with new code)
```

CDK enforces this ordering via `addDependency()`. New Lambda code only deploys AFTER migrations succeed.

### Rollback

If migration Lambda fails → CDK Custom Resource fails → CloudFormation rolls back the entire ComputeStack deployment. No new Lambda code deployed with incompatible schema.

If migration succeeds but new Lambda code fails → CloudFormation rolls back Lambda to previous version, but migration is already applied. **This is acceptable** because:

1. Drizzle migrations should be backward-compatible (add columns, never remove)
2. The old Lambda code can still work with the new schema
3. Forward-only migration is standard practice

## GitHub Actions Integration

```yaml
# .github/workflows/deploy.yml
deploy-staging:
  steps:
    - uses: aws-actions/configure-aws-credentials@v4
      with:
        role-to-assume: arn:aws:iam::${{ secrets.STAGING_ACCOUNT_ID }}:role/github-actions-deploy
        aws-region: us-east-1

    - run: npm ci
    - run: npx nx run-many -t build # Build all apps
    - run: npx nx deploy aws-cdk -- --context env=staging

deploy-production:
  needs: deploy-staging
  environment: production # GitHub Environment with approval gate
  steps:
    - uses: aws-actions/configure-aws-credentials@v4
      with:
        role-to-assume: arn:aws:iam::${{ secrets.PRODUCTION_ACCOUNT_ID }}:role/github-actions-deploy
        aws-region: us-east-1

    - run: npm ci
    - run: npx nx run-many -t build
    - run: npx nx deploy aws-cdk -- --context env=production
```

### PR Preview with `cdk diff`

```yaml
# On pull_request
cdk-diff:
  steps:
    - uses: aws-actions/configure-aws-credentials@v4
      with:
        role-to-assume: ${{ secrets.STAGING_DEPLOY_ROLE }}
    - run: npm ci && npx nx build aws-cdk
    - run: npx nx diff aws-cdk -- --context env=staging
    # Post diff output as PR comment
```

## Provisioned Concurrency Per Environment

Provisioned concurrency eliminates Lambda cold starts by keeping warm instances. Analysis:

| Factor            | Staging                                       | Production                                                   |
| ----------------- | --------------------------------------------- | ------------------------------------------------------------ |
| Need              | Low — internal use, latency tolerance high    | Higher — public users expect <500ms first response           |
| API Lambda        | `0` — no provisioned concurrency              | `5-10` — based on expected concurrent users                  |
| Ledger Processor  | `0` — Rust cold start ~100ms is acceptable    | `0` — Rust cold start is fast enough, triggered by S3 events |
| Event Interpreter | `0` — runs every 5 min, cold start irrelevant | `0` — scheduled, not latency-sensitive                       |
| Cost              | $0                                            | ~$35-70/month per provisioned instance                       |

**Recommendation:** Only API Lambda in production needs provisioned concurrency. Set `apiLambdaProvisionedConcurrency` in `EnvironmentConfig`. Start with 5, adjust based on CloudWatch p99 latency.

Per ADR-0002: Rust Ledger Processor does NOT need provisioned concurrency (100-300ms cold start vs 500-1500ms Node.js).

## S3 Lifecycle Rules Per Environment

Lifecycle rules on `stellar-ledger-data` bucket control XDR artifact retention. Per architecture docs:

| Environment | Retention | Rationale                                                            |
| ----------- | --------- | -------------------------------------------------------------------- |
| Staging     | 7 days    | Enough for testing and replay; shorter reduces storage cost          |
| Production  | 30 days   | Supports incident investigation, XDR replay, and backfill validation |

Implementation in CDK:

```typescript
const ledgerBucket = new s3.Bucket(this, 'LedgerData', {
  lifecycleRules: [
    {
      expiration: cdk.Duration.days(config.ledgerDataRetentionDays),
      // 7 for staging, 30 for production
    },
  ],
});
```

The `api-docs` bucket has no lifecycle rules — documentation assets are persistent.

## Staging Frontend Password Protection

**Recommendation: CloudFront Function with Basic Auth**

Lambda@Edge is overkill — CloudFront Functions are cheaper (~1/6 cost), faster (sub-ms), and sufficient for Basic Auth.

```typescript
// CloudFront Function for Basic Auth
const authFunction = new cloudfront.Function(this, 'BasicAuth', {
  code: cloudfront.FunctionCode.fromInline(`
    function handler(event) {
      var request = event.request;
      var headers = request.headers;
      var authString = 'Basic ' + '<base64-encoded-credentials>';
      if (!headers.authorization || headers.authorization.value !== authString) {
        return {
          statusCode: 401,
          headers: { 'www-authenticate': { value: 'Basic realm="Staging"' } },
        };
      }
      return request;
    }
  `),
  runtime: cloudfront.FunctionRuntime.JS_2_0,
});
```

**Password source:** The base64-encoded credentials are injected at synth time from `config.stagingPassword` (which references a Secrets Manager ARN). The CloudFront Function itself is deployed with the resolved credential — it does NOT call Secrets Manager at runtime (CloudFront Functions can't make network calls).

**Alternative considered:** Lambda@Edge with Secrets Manager lookup — runtime resolution, more secure but adds 50-100ms latency per request and costs 3x more. Rejected for a staging-only protection mechanism.

**Optional controls:** IP allowlists via WAF WebACL rules for additional staging access restriction (configurable via `EnvironmentConfig`).
