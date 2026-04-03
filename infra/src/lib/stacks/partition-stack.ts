import * as cdk from 'aws-cdk-lib';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
import * as cr from 'aws-cdk-lib/custom-resources';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as events from 'aws-cdk-lib/aws-events';
import * as targets from 'aws-cdk-lib/aws-events-targets';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import { RustFunction } from 'cargo-lambda-cdk';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface PartitionStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  readonly vpc: ec2.IVpc;
  readonly lambdaSecurityGroup: ec2.ISecurityGroup;
  readonly dbSecret: secretsmanager.ISecret;
  readonly dbProxyEndpoint: string;
  readonly cargoWorkspacePath: string;
}

/**
 * Partition management stack for the Soroban Block Explorer.
 *
 * Creates and maintains PostgreSQL table partitions via a Lambda that:
 * 1. Runs on every deployment (CDK custom resource)
 * 2. Runs monthly via EventBridge schedule
 * 3. Publishes CloudWatch metrics for monitoring
 *
 * Dependency ordering (enforced in app.ts):
 *   NetworkStack → RdsStack → MigrationStack → PartitionStack → ComputeStack
 */
export class PartitionStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: PartitionStackProps) {
    super(scope, id, props);

    const {
      config,
      vpc,
      lambdaSecurityGroup,
      dbSecret,
      dbProxyEndpoint,
      cargoWorkspacePath,
    } = props;

    const metricsNamespace = `SorobanExplorer/${config.envName}/Partitions`;

    // ---------------------
    // Partition Lambda
    // ---------------------
    const partitionFn = new RustFunction(this, 'PartitionFunction', {
      functionName: `${config.envName}-soroban-explorer-partition-mgmt`,
      manifestPath: cargoWorkspacePath,
      binaryName: 'db-partition-mgmt',
      architecture: lambda.Architecture.ARM_64,
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
      securityGroups: [lambdaSecurityGroup],
      memorySize: 256,
      timeout: cdk.Duration.minutes(5),
      logRetention: logs.RetentionDays.ONE_MONTH,
      environment: {
        RDS_PROXY_ENDPOINT: dbProxyEndpoint,
        SECRET_ARN: dbSecret.secretArn,
        ENV_NAME: config.envName,
        RUST_LOG: 'info',
      },
    });

    dbSecret.grantRead(partitionFn);

    partitionFn.addToRolePolicy(
      new iam.PolicyStatement({
        actions: ['cloudwatch:PutMetricData'],
        resources: ['*'],
        conditions: {
          StringEquals: { 'cloudwatch:namespace': metricsNamespace },
        },
      })
    );

    // ---------------------
    // CDK Custom Resource (runs on deploy)
    // ---------------------
    const provider = new cr.Provider(this, 'PartitionProvider', {
      onEventHandler: partitionFn,
      logRetention: logs.RetentionDays.ONE_MONTH,
    });

    new cdk.CustomResource(this, 'EnsurePartitions', {
      serviceToken: provider.serviceToken,
      properties: {
        // Force re-invocation on every deploy
        partitionVersion: Date.now().toString(),
      },
    });

    // ---------------------
    // EventBridge Schedule (monthly)
    // ---------------------
    new events.Rule(this, 'MonthlyPartitionRule', {
      ruleName: `${config.envName}-partition-monthly`,
      description: 'Create future partitions on 1st of each month',
      schedule: events.Schedule.cron({
        minute: '0',
        hour: '2',
        day: '1',
        month: '*',
        year: '*',
      }),
      targets: [new targets.LambdaFunction(partitionFn)],
    });

    // ---------------------
    // CloudWatch Alarms
    // ---------------------
    const timePartitionedTables = [
      'soroban_invocations',
      'soroban_events',
      'liquidity_pool_snapshots',
    ];

    for (const table of timePartitionedTables) {
      new cloudwatch.Alarm(this, `FuturePartitions-${table}`, {
        alarmName: `${config.envName}-partition-future-low-${table}`,
        alarmDescription: `Fewer than 2 future partitions for ${table}`,
        metric: new cloudwatch.Metric({
          namespace: metricsNamespace,
          metricName: 'FuturePartitionCount',
          dimensionsMap: { Table: table },
          period: cdk.Duration.days(1),
          statistic: 'Minimum',
        }),
        threshold: 2,
        comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
        evaluationPeriods: 1,
        treatMissingData: cloudwatch.TreatMissingData.BREACHING,
      });
    }

    new cloudwatch.Alarm(this, 'OperationsRangeHigh', {
      alarmName: `${config.envName}-partition-operations-range-high`,
      alarmDescription:
        'Operations partition range >80% consumed — create next range',
      metric: new cloudwatch.Metric({
        namespace: metricsNamespace,
        metricName: 'OperationsRangeUsagePercent',
        period: cdk.Duration.days(1),
        statistic: 'Maximum',
      }),
      threshold: 80,
      comparisonOperator: cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
      evaluationPeriods: 1,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    });

    new cloudwatch.Alarm(this, 'PartitionLambdaErrors', {
      alarmName: `${config.envName}-partition-lambda-errors`,
      alarmDescription: 'Partition management Lambda invocation errors',
      metric: partitionFn.metricErrors({
        period: cdk.Duration.days(1),
        statistic: 'Sum',
      }),
      threshold: 0,
      comparisonOperator: cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
      evaluationPeriods: 1,
      treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
    });

    // ---------------------
    // Tags
    // ---------------------
    cdk.Tags.of(this).add('Project', 'soroban-block-explorer');
    cdk.Tags.of(this).add('Environment', config.envName);
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
  }
}
