import * as cdk from 'aws-cdk-lib';

import type { EnvironmentConfig } from './types.js';
import { NetworkStack } from './stacks/network-stack.js';
import { RdsStack } from './stacks/rds-stack.js';
import { LedgerBucketStack } from './stacks/ledger-bucket-stack.js';

export function createApp(config: EnvironmentConfig): void {
  const app = new cdk.App();

  const env: cdk.Environment = {
    account: process.env['CDK_DEFAULT_ACCOUNT'],
    region: config.awsRegion,
  };

  const prefix = `Explorer-${config.envName}`;

  const network = new NetworkStack(app, `${prefix}-Network`, { env, config });

  new RdsStack(app, `${prefix}-Rds`, {
    env,
    config,
    vpc: network.vpc,
    lambdaSecurityGroup: network.lambdaSecurityGroup,
    ecsSecurityGroup: network.ecsSecurityGroup,
  });

  new LedgerBucketStack(app, `${prefix}-LedgerBucket`, { env, config });

  app.synth();
}
