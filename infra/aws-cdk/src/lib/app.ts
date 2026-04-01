import * as cdk from 'aws-cdk-lib';

import type { EnvironmentConfig } from './types.js';
import { NetworkStack } from './stacks/network-stack.js';
import { RdsStack } from './stacks/rds-stack.js';
import { LedgerBucketStack } from './stacks/ledger-bucket-stack.js';
import { ComputeStack } from './stacks/compute-stack.js';

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

  new ComputeStack(app, `${prefix}-Compute`, {
    env,
    config,
    vpc: network.vpc,
    lambdaSecurityGroup: network.lambdaSecurityGroup,
    dbSecret: rds.dbSecret,
    dbProxyEndpoint: rds.dbProxy
      ? rds.dbProxy.endpoint
      : rds.dbInstance.instanceEndpoint.hostname,
    ledgerBucket: ledgerBucket.bucket,
    cargoWorkspacePath,
  });

  app.synth();
}
