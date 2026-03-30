#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib';

import { resolveConfig } from '../lib/config/index.js';
import { NetworkStack } from '../lib/stacks/network-stack.js';

const app = new cdk.App();

const envName = app.node.tryGetContext('env') as string | undefined;
const config = resolveConfig(envName);

// Account from CDK_DEFAULT_ACCOUNT (set by `cdk deploy` or CI credentials).
// Intentionally not hardcoded — this is an open-source repo.
const account = process.env['CDK_DEFAULT_ACCOUNT'];
if (!account) {
  throw new Error(
    'CDK_DEFAULT_ACCOUNT is not set. Run via `cdk deploy` or set AWS credentials.'
  );
}

const env: cdk.Environment = { account, region: config.awsRegion };
const prefix = `Explorer-${config.envName}`;

new NetworkStack(app, `${prefix}-Network`, { env, config });

app.synth();
