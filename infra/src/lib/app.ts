import * as cdk from 'aws-cdk-lib';

import type { EnvironmentConfig } from './types.js';
import { NetworkStack } from './stacks/network-stack.js';
import { RdsStack } from './stacks/rds-stack.js';
import { LedgerBucketStack } from './stacks/ledger-bucket-stack.js';
import { ComputeStack } from './stacks/compute-stack.js';
import { MigrationStack } from './stacks/migration-stack.js';
import { ApiGatewayStack } from './stacks/api-gateway-stack.js';
import { IngestionStack } from './stacks/ingestion-stack.js';

export interface CreateAppOptions {
  readonly config: EnvironmentConfig;
  /** Absolute path to the directory containing the root Cargo.toml workspace. */
  readonly cargoWorkspacePath: string;
}

export function createApp({
  config,
  cargoWorkspacePath,
}: CreateAppOptions): void {
  const app = new cdk.App();

  const env: cdk.Environment = {
    account: process.env['CDK_DEFAULT_ACCOUNT'],
    region: config.awsRegion,
  };

  const prefix = `Explorer-${config.envName}`;

  const network = new NetworkStack(app, `${prefix}-Network`, { env, config });

  const rds = new RdsStack(app, `${prefix}-Rds`, {
    env,
    config,
    vpc: network.vpc,
    lambdaSecurityGroup: network.lambdaSecurityGroup,
    ecsSecurityGroup: network.ecsSecurityGroup,
  });

  const ledgerBucket = new LedgerBucketStack(app, `${prefix}-LedgerBucket`, {
    env,
    config,
  });

  const dbProxyEndpoint = rds.dbProxy
    ? rds.dbProxy.endpoint
    : rds.dbInstance.instanceEndpoint.hostname;

  const migration = new MigrationStack(app, `${prefix}-Migration`, {
    env,
    config,
    vpc: network.vpc,
    lambdaSecurityGroup: network.lambdaSecurityGroup,
    dbSecret: rds.dbSecret,
    dbProxyEndpoint,
    cargoWorkspacePath,
  });
  migration.addDependency(rds);

  const compute = new ComputeStack(app, `${prefix}-Compute`, {
    env,
    config,
    vpc: network.vpc,
    lambdaSecurityGroup: network.lambdaSecurityGroup,
    dbSecret: rds.dbSecret,
    dbProxyEndpoint,
    ledgerBucketArn: ledgerBucket.bucket.bucketArn,
    ledgerBucketName: ledgerBucket.bucket.bucketName,
    cargoWorkspacePath,
  });
  compute.addDependency(migration);

  new IngestionStack(app, `${prefix}-Ingestion`, {
    env,
    config,
    vpc: network.vpc,
    ecsSecurityGroup: network.ecsSecurityGroup,
    ledgerBucketArn: ledgerBucket.bucket.bucketArn,
    ledgerBucketName: ledgerBucket.bucket.bucketName,
  });
  // CDK auto-detects dependencies from cross-stack references
  // (vpc, ecsSecurityGroup, bucket ARN/name).

  const apiGateway = new ApiGatewayStack(app, `${prefix}-ApiGateway`, {
    env,
    config,
    apiFunction: compute.apiFunction,
    // wafWebAclArn: delivery.wafWebAclArn,  // task 0035
  });
  apiGateway.addDependency(compute);

  app.synth();
}
