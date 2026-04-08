import * as cdk from 'aws-cdk-lib';

import type { CicdConfig } from './types.js';
import { CicdStack } from './stacks/cicd-stack.js';

export function createCicdApp(config: CicdConfig): void {
  const app = new cdk.App();

  const env: cdk.Environment = {
    account: process.env['CDK_DEFAULT_ACCOUNT'],
    region: config.awsRegion,
  };

  new CicdStack(app, 'Explorer-Cicd', {
    env,
    githubRepo: config.githubRepo,
    awsRegion: config.awsRegion,
  });

  app.synth();
}
