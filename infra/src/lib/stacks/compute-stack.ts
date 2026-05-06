import * as cdk from 'aws-cdk-lib';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as lambdaDestinations from 'aws-cdk-lib/aws-lambda-destinations';
import * as lambdaEventSources from 'aws-cdk-lib/aws-lambda-event-sources';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as s3n from 'aws-cdk-lib/aws-s3-notifications';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import * as sqs from 'aws-cdk-lib/aws-sqs';
import { RustFunction } from 'cargo-lambda-cdk';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

const DLQ_RETENTION_DAYS = 14;

export interface ComputeStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  readonly vpc: ec2.IVpc;
  readonly lambdaSecurityGroup: ec2.ISecurityGroup;
  readonly dbSecret: secretsmanager.ISecret;
  readonly dbProxyEndpoint: string;
  readonly ledgerBucketArn: string;
  readonly ledgerBucketName: string;
  readonly cargoWorkspacePath: string;
}

/**
 * Compute layer for the Soroban Block Explorer.
 *
 * Contains two Rust Lambda functions built via cargo-lambda-cdk:
 * - API Lambda (axum): serves REST API, reads from PostgreSQL
 * - Ledger Processor Lambda (indexer): processes S3 PutObject events,
 *   parses XDR, writes to PostgreSQL
 *
 * Both run on ARM64/Graviton2 in VPC private subnets with the Lambda
 * security group. Failed processor invocations route to an SQS DLQ.
 */
export class ComputeStack extends cdk.Stack {
  readonly apiFunction: lambda.IFunction;
  readonly processorFunction: lambda.IFunction;
  readonly deadLetterQueue: sqs.IQueue;
  /** Type-1 enrichment DLQ (task 0191). Exposed so the
   *  observability layer can attach the depth alarm. */
  readonly enrichmentDlq: sqs.IQueue;
  /** Type-1 enrichment worker Lambda (task 0191). Exposed so the
   *  observability layer can attach error-rate / metric alarms. */
  readonly enrichmentWorkerFunction: lambda.IFunction;

  constructor(scope: Construct, id: string, props: ComputeStackProps) {
    super(scope, id, props);

    const {
      config,
      vpc,
      lambdaSecurityGroup,
      dbSecret,
      dbProxyEndpoint,
      ledgerBucketArn,
      ledgerBucketName,
      cargoWorkspacePath,
    } = props;

    // Import the ledger bucket by name/ARN to break the cross-stack
    // cyclic dependency that occurs with direct IBucket references.
    // LedgerBucketStack owns the bucket; ComputeStack only needs to
    // read from it and add an event notification.
    const ledgerBucket = s3.Bucket.fromBucketAttributes(this, 'LedgerBucket', {
      bucketArn: ledgerBucketArn,
      bucketName: ledgerBucketName,
    });

    const apiLogGroup = new logs.LogGroup(this, 'ApiLogGroup', {
      logGroupName: `/aws/lambda/${config.envName}-soroban-explorer-api`,
      retention: logs.RetentionDays.ONE_MONTH,
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });

    const processorLogGroup = new logs.LogGroup(this, 'ProcessorLogGroup', {
      logGroupName: `/aws/lambda/${config.envName}-soroban-explorer-indexer`,
      retention: logs.RetentionDays.ONE_MONTH,
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });

    const enrichmentWorkerLogGroup = new logs.LogGroup(
      this,
      'EnrichmentWorkerLogGroup',
      {
        logGroupName: `/aws/lambda/${config.envName}-soroban-explorer-enrichment-worker`,
        retention: logs.RetentionDays.ONE_MONTH,
        removalPolicy: cdk.RemovalPolicy.DESTROY,
      }
    );

    const sharedLambdaProps = {
      architecture: lambda.Architecture.ARM_64,
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
      securityGroups: [lambdaSecurityGroup],
      tracing: lambda.Tracing.ACTIVE,
    };

    const sharedEnv = {
      RDS_PROXY_ENDPOINT: dbProxyEndpoint,
      SECRET_ARN: dbSecret.secretArn,
      ENV_NAME: config.envName,
      // Task 0160 — indexer derives SAC `contract_id` deterministically
      // (`SHA256(network_id || XDR(ContractIdPreimage))`) and panics if
      // this passphrase is missing. Same value used by Galexie partition
      // mapping in ingestion-stack — single source of truth.
      STELLAR_NETWORK_PASSPHRASE: config.stellarNetworkPassphrase,
    };

    // ---------------------
    // SQS Dead-Letter Queue
    // ---------------------
    // Created first because the processor Lambda references it.
    // Receives S3 event records that exhausted Lambda async retries.
    // Messages contain bucket/key for manual replay.
    const dlq = new sqs.Queue(this, 'ProcessorDlq', {
      queueName: `${config.envName}-ledger-processor-dlq`,
      retentionPeriod: cdk.Duration.days(DLQ_RETENTION_DAYS),
    });
    this.deadLetterQueue = dlq;

    // ---------------------
    // Type-1 Enrichment Queue (task 0191)
    // ---------------------
    // Indexer publishes one message per asset that needs runtime
    // enrichment (icon today; LP analytics later) after each ledger
    // commit. The worker Lambda below consumes the queue, fetches the
    // SEP-1 toml, and writes assets.icon_url. Standard queue (at-least-
    // once delivery is fine; the worker is idempotent on
    // assets.icon_url IS NULL but is also designed to handle
    // duplicate-as-refresh per task 0191).
    const enrichmentDlq = new sqs.Queue(this, 'EnrichmentDlq', {
      queueName: `${config.envName}-enrichment-dlq`,
      retentionPeriod: cdk.Duration.days(DLQ_RETENTION_DAYS),
    });

    const enrichmentQueue = new sqs.Queue(this, 'EnrichmentQueue', {
      queueName: `${config.envName}-enrichment`,
      retentionPeriod: cdk.Duration.days(DLQ_RETENTION_DAYS),
      // Visibility timeout must exceed the worker's per-record budget.
      // Worker is one HTTP fetch (~2 s timeout) + one UPDATE — pad to
      // 60 s so a slow batch never duplicates work mid-flight.
      visibilityTimeout: cdk.Duration.seconds(60),
      deadLetterQueue: {
        queue: enrichmentDlq,
        maxReceiveCount: 3,
      },
    });
    this.enrichmentDlq = enrichmentDlq;

    // Depth alarm + dashboard widget for `enrichmentDlq` live in
    // `CloudWatchStack` alongside the other alarms — same pattern as
    // the ledger-processor DLQ. ComputeStack only owns the queue
    // itself; observability is wired in the dedicated stack.

    // ---------------------
    // API Lambda
    // ---------------------
    const apiFunction = new RustFunction(this, 'ApiFunction', {
      functionName: `${config.envName}-soroban-explorer-api`,
      manifestPath: cargoWorkspacePath,
      binaryName: 'api',
      ...sharedLambdaProps,
      logGroup: apiLogGroup,
      memorySize: config.apiLambdaMemory,
      timeout: cdk.Duration.seconds(config.apiLambdaTimeout),
      environment: {
        ...sharedEnv,
        // lambda_http prepends the API Gateway stage name to the path by default
        // (e.g. /health becomes /staging/health), breaking axum route matching.
        // This env var tells lambda_http to skip the stage prefix.
        AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH: 'true',
        // Advertised base URL for the OpenAPI `servers` block so the
        // spec generated at runtime (task 0042) carries the correct
        // hostname per environment. Consumed by api::config::AppConfig.
        API_BASE_URL: `https://${config.apiDomainName}`,
      },
    });
    this.apiFunction = apiFunction;

    // ---------------------
    // Ledger Processor Lambda
    // ---------------------
    const processorFunction = new RustFunction(this, 'ProcessorFunction', {
      functionName: `${config.envName}-soroban-explorer-indexer`,
      manifestPath: cargoWorkspacePath,
      binaryName: 'indexer',
      ...sharedLambdaProps,
      logGroup: processorLogGroup,
      memorySize: config.indexerLambdaMemory,
      timeout: cdk.Duration.seconds(config.indexerLambdaTimeout),
      // Limit concurrency to avoid exhausting RDS max_connections (~87 on t4g.micro).
      // Each instance holds 1 DB connection (pool.rs max_connections=1).
      // 20 concurrent is sufficient for Galexie's ~12 files/min throughput.
      reservedConcurrentExecutions: config.indexerLambdaConcurrency,
      environment: {
        ...sharedEnv,
        BUCKET_NAME: ledgerBucket.bucketName,
        RUST_LOG: 'info',
        // Task 0191 — indexer emits enrichment messages here after
        // each ledger commit. When the env var is unset (local dev /
        // tests) the publisher silently no-ops.
        ENRICHMENT_QUEUE_URL: enrichmentQueue.queueUrl,
      },
    });
    this.processorFunction = processorFunction;

    // Retry failed async invocations twice, then send to DLQ.
    new lambda.EventInvokeConfig(this, 'ProcessorInvokeConfig', {
      function: processorFunction,
      retryAttempts: config.indexerLambdaRetryAttempts,
      onFailure: new lambdaDestinations.SqsDestination(dlq),
    });

    // S3 PutObject trigger — fires the processor for each new ledger file.
    // Filtered to .xdr.zst suffix to avoid triggering on non-ledger objects
    // (e.g. metadata files, logs). Galexie writes ledger files as:
    //   {hex}--{start}-{end}/{hex}--{start}[-{end}].xdr.zst
    // This suffix MUST match parse_s3_key() in crates/xdr-parser/src/lib.rs.
    // CDK automatically adds Lambda invoke permission for S3.
    // Skip when concurrency=0 — avoids queuing events for a throttled Lambda.
    if (config.indexerLambdaConcurrency > 0) {
      ledgerBucket.addEventNotification(
        s3.EventType.OBJECT_CREATED,
        new s3n.LambdaDestination(processorFunction),
        { suffix: '.xdr.zst' }
      );
    }

    // ---------------------
    // Type-1 Enrichment Worker Lambda (task 0191)
    // ---------------------
    const enrichmentWorkerFunction = new RustFunction(
      this,
      'EnrichmentWorkerFunction',
      {
        functionName: `${config.envName}-soroban-explorer-enrichment-worker`,
        manifestPath: cargoWorkspacePath,
        binaryName: 'enrichment-worker',
        ...sharedLambdaProps,
        logGroup: enrichmentWorkerLogGroup,
        memorySize: config.enrichmentWorkerLambdaMemory,
        timeout: cdk.Duration.seconds(config.enrichmentWorkerLambdaTimeout),
        // Polite to issuer servers and bounded against accidental RDS
        // exhaustion. Mirror the indexer concurrency pattern: 0 in
        // staging (disabled), low single-digit in production. Bursts
        // of 200 parallel HTTPS requests to one issuer's host would
        // look indistinguishable from a DDoS attack.
        reservedConcurrentExecutions: config.enrichmentWorkerLambdaConcurrency,
        environment: {
          ...sharedEnv,
          RUST_LOG: 'info',
        },
      }
    );

    // SQS event source mapping. ReportBatchItemFailures lets the worker
    // ack only the records it successfully processed — failed records
    // redeliver up to maxReceiveCount and then land in the DLQ.
    enrichmentWorkerFunction.addEventSource(
      new lambdaEventSources.SqsEventSource(enrichmentQueue, {
        batchSize: 10,
        maxBatchingWindow: cdk.Duration.seconds(5),
        reportBatchItemFailures: true,
      })
    );
    this.enrichmentWorkerFunction = enrichmentWorkerFunction;

    // ---------------------
    // IAM Grants
    // ---------------------
    dbSecret.grantRead(apiFunction);
    dbSecret.grantRead(processorFunction);
    dbSecret.grantRead(enrichmentWorkerFunction);
    ledgerBucket.grantRead(processorFunction);
    processorFunction.addToRolePolicy(
      new iam.PolicyStatement({
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: {
            'cloudwatch:namespace': 'SorobanBlockExplorer/Indexer',
          },
        },
      })
    );

    // Indexer publishes enrichment messages.
    enrichmentQueue.grantSendMessages(processorFunction);
    // Worker reads + deletes from the queue (grantConsumeMessages also
    // covers ChangeMessageVisibility for partial-batch failures).
    enrichmentQueue.grantConsumeMessages(enrichmentWorkerFunction);

    // ---------------------
    // Tags
    // ---------------------
    cdk.Tags.of(this).add('Project', 'soroban-block-explorer');
    cdk.Tags.of(this).add('Environment', config.envName);
    cdk.Tags.of(this).add('ManagedBy', 'cdk');

    // ---------------------
    // Outputs
    // ---------------------
    new cdk.CfnOutput(this, 'ApiLambdaArn', {
      value: apiFunction.functionArn,
    });
    new cdk.CfnOutput(this, 'ProcessorLambdaArn', {
      value: processorFunction.functionArn,
    });
    new cdk.CfnOutput(this, 'DlqUrl', {
      value: dlq.queueUrl,
    });
  }
}
