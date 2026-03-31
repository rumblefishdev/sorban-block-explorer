import * as cdk from 'aws-cdk-lib';

import type { EnvironmentConfig } from './types.js';
import { NetworkStack } from './stacks/network-stack.js';

export function createApp(config: EnvironmentConfig): void {
  const app = new cdk.App();

  const env: cdk.Environment = {
    account: process.env['CDK_DEFAULT_ACCOUNT'],
    region: config.awsRegion,
  };

  const prefix = `Explorer-${config.envName}`;

  new NetworkStack(app, `${prefix}-Network`, { env, config });

  // Future stacks will be added here by their respective tasks.

  app.synth();
}
