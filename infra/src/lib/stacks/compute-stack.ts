import * as cdk from 'aws-cdk-lib';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as lambdaDestinations from 'aws-cdk-lib/aws-lambda-destinations';
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
      environment: {
        ...sharedEnv,
        BUCKET_NAME: ledgerBucket.bucketName,
        RUST_LOG: 'info',
      },
    });
    this.processorFunction = processorFunction;

    // Retry failed async invocations twice, then send to DLQ.
    new lambda.EventInvokeConfig(this, 'ProcessorInvokeConfig', {
      function: processorFunction,
      retryAttempts: 2,
      onFailure: new lambdaDestinations.SqsDestination(dlq),
    });

    // S3 PutObject trigger — fires the processor for each new ledger file.
    // Filtered to .xdr.zst suffix to avoid triggering on non-ledger objects
    // (e.g. metadata files, logs). Galexie writes ledger files as:
    //   {hex}--{start}-{end}/{hex}--{start}[-{end}].xdr.zst
    // This suffix MUST match parse_s3_key() in crates/xdr-parser/src/lib.rs.
    // CDK automatically adds Lambda invoke permission for S3.
    ledgerBucket.addEventNotification(
      s3.EventType.OBJECT_CREATED,
      new s3n.LambdaDestination(processorFunction),
      { suffix: '.xdr.zst' }
    );

    // ---------------------
    // IAM Grants
    // ---------------------
    dbSecret.grantRead(apiFunction);
    dbSecret.grantRead(processorFunction);
    ledgerBucket.grantRead(processorFunction);

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
