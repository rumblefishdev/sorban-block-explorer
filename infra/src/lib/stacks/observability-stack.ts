import * as cdk from 'aws-cdk-lib';
import * as xray from 'aws-cdk-lib/aws-xray';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface ObservabilityStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * X-Ray sampling rules for the Soroban Block Explorer.
 *
 * Sampling rates are environment-specific:
 * - Staging: 100% (all requests traced for debugging)
 * - Production: 5% + 1 trace/sec reservoir (cost-efficient)
 *
 * X-Ray active tracing is enabled on:
 * - API Gateway (api-gateway-stack.ts: tracingEnabled)
 * - Lambda functions (compute-stack.ts: tracing: ACTIVE)
 *
 * RDS Proxy does not support X-Ray natively. Database call latency
 * is captured as subsegments in Lambda traces via the AWS SDK.
 */
export class ObservabilityStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: ObservabilityStackProps) {
    super(scope, id, props);

    const { config } = props;

    // ---------------------
    // X-Ray Sampling Rule
    // ---------------------
    // Applies to all services in this environment. The rule name includes
    // the environment to avoid collisions when staging and production
    // share the same AWS account.
    new xray.CfnSamplingRule(this, 'SamplingRule', {
      samplingRule: {
        ruleName: `${config.envName}-soroban-explorer`,
        priority: 1000,
        fixedRate: config.xraySamplingRate,
        reservoirSize: config.xrayReservoirSize,
        serviceName: `*`,
        serviceType: `*`,
        host: `*`,
        httpMethod: `*`,
        urlPath: `*`,
        resourceArn: `*`,
        version: 1,
      },
    });

    // ---------------------
    // Tags
    // ---------------------
    cdk.Tags.of(this).add('Project', 'soroban-block-explorer');
    cdk.Tags.of(this).add('Environment', config.envName);
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
  }
}
