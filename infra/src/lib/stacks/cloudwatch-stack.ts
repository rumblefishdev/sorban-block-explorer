import * as cdk from 'aws-cdk-lib';
import * as cloudwatch from 'aws-cdk-lib/aws-cloudwatch';
import * as chatbot from 'aws-cdk-lib/aws-chatbot';
import * as cloudwatchActions from 'aws-cdk-lib/aws-cloudwatch-actions';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as rds from 'aws-cdk-lib/aws-rds';
import * as sns from 'aws-cdk-lib/aws-sns';
import * as sqs from 'aws-cdk-lib/aws-sqs';
import * as apigateway from 'aws-cdk-lib/aws-apigateway';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface CloudWatchStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  readonly apiFunction: lambda.IFunction;
  readonly processorFunction: lambda.IFunction;
  readonly deadLetterQueue: sqs.IQueue;
  /** Type-1 enrichment DLQ (task 0191) — alarmed on depth > 0. */
  readonly enrichmentDlq: sqs.IQueue;
  /** Type-1 enrichment worker Lambda (task 0191) — error-rate alarm. */
  readonly enrichmentWorkerFunction: lambda.IFunction;
  readonly rdsInstance: rds.IDatabaseInstance;
  readonly restApi: apigateway.RestApi;
}

/**
 * Observability layer — CloudWatch dashboards and alarms.
 *
 * Creates:
 * - One SNS topic per environment for alarm notifications
 * - AWS Chatbot SlackChannelConfiguration subscribing the topic to a Slack channel
 * - 6 alarms covering Galexie ingestion lag, Processor error rate, DLQ depth,
 *   RDS CPU, RDS free storage, and API Gateway 5xx rate
 * - A CloudWatch dashboard with Ingestion / API / Resources sections
 *
 * All alarm thresholds are env-configurable via EnvironmentConfig.
 *
 * Prerequisites (one-time manual step):
 * Authorize the Slack workspace in the AWS Console under AWS Chatbot before
 * deploying. Without this the SlackChannelConfiguration will fail to create.
 *
 * Note: RDS storage alarm uses an absolute threshold (GiB) derived from
 * rdsStorageThresholdPct × dbAllocatedStorage. CloudWatch RDS metrics
 * report FreeStorageSpace in bytes, so the threshold is converted accordingly.
 */
export class CloudWatchStack extends cdk.Stack {
  readonly alarmTopic: sns.Topic;

  constructor(scope: Construct, id: string, props: CloudWatchStackProps) {
    super(scope, id, props);

    const {
      config,
      apiFunction,
      processorFunction,
      deadLetterQueue,
      enrichmentDlq,
      enrichmentWorkerFunction,
      rdsInstance,
      restApi,
    } = props;

    // ---------------------
    // SNS Topic
    // ---------------------
    const alarmTopic = new sns.Topic(this, 'AlarmTopic', {
      topicName: `${config.envName}-soroban-explorer-alarms`,
      displayName: `${config.envName} Soroban Explorer Alarms`,
    });
    this.alarmTopic = alarmTopic;

    // ---------------------
    // AWS Chatbot — Slack channel
    // Prerequisite: authorize the Slack workspace in the AWS Console under
    // AWS Chatbot (one-time manual step) before running cdk deploy.
    // ---------------------
    new chatbot.SlackChannelConfiguration(this, 'SlackChannel', {
      slackChannelConfigurationName: `${config.envName}-soroban-explorer-alarms`,
      slackWorkspaceId: config.slackWorkspaceId,
      slackChannelId: config.slackChannelId,
      notificationTopics: [alarmTopic],
      role: new iam.Role(this, 'ChatbotRole', {
        assumedBy: new iam.ServicePrincipal('chatbot.amazonaws.com'),
        managedPolicies: [
          iam.ManagedPolicy.fromAwsManagedPolicyName(
            'CloudWatchReadOnlyAccess'
          ),
        ],
      }),
    });

    const alarmAction = new cloudwatchActions.SnsAction(alarmTopic);

    // ---------------------
    // Helper: attach both alarm and ok actions
    // ---------------------
    const withActions = (alarm: cloudwatch.Alarm): cloudwatch.Alarm => {
      alarm.addAlarmAction(alarmAction);
      alarm.addOkAction(alarmAction);
      return alarm;
    };

    // ---------------------
    // Alarm 1: Galexie ingestion lag
    // Fires when Ledger Processor has 0 invocations for N consecutive minutes.
    // This is a proxy for "Galexie stopped writing to S3".
    // ---------------------
    withActions(
      new cloudwatch.Alarm(this, 'GalexieLagAlarm', {
        alarmName: `${config.envName}-galexie-ingestion-lag`,
        alarmDescription:
          'Ledger Processor invocations dropped to 0 — Galexie may have stopped writing to S3.',
        metric: processorFunction.metricInvocations({
          period: cdk.Duration.minutes(1),
          statistic: cloudwatch.Stats.SUM,
        }),
        threshold: 1,
        comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
        evaluationPeriods: config.galexieLagMinutes,
        treatMissingData: cloudwatch.TreatMissingData.BREACHING,
      })
    );

    // ---------------------
    // Alarm 2: Ledger Processor error rate
    // Uses a MathExpression: errors / invocations > threshold.
    // ---------------------
    const processorErrors = processorFunction.metricErrors({
      period: cdk.Duration.minutes(5),
      statistic: cloudwatch.Stats.SUM,
    });
    const processorInvocations = processorFunction.metricInvocations({
      period: cdk.Duration.minutes(5),
      statistic: cloudwatch.Stats.SUM,
    });
    withActions(
      new cloudwatch.Alarm(this, 'ProcessorErrorRateAlarm', {
        alarmName: `${config.envName}-ledger-processor-error-rate`,
        alarmDescription:
          'Ledger Processor error rate exceeded threshold — ledgers may be failing to index.',
        metric: new cloudwatch.MathExpression({
          expression: 'errors / invocations',
          usingMetrics: {
            errors: processorErrors,
            invocations: processorInvocations,
          },
          period: cdk.Duration.minutes(5),
          label: 'Error Rate',
        }),
        threshold: config.processorErrorRateThreshold,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        evaluationPeriods: 1,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      })
    );

    // ---------------------
    // Alarm 3: RDS CPU
    // Fires when CPU > threshold sustained for 5 consecutive 1-min periods.
    // ---------------------
    withActions(
      new cloudwatch.Alarm(this, 'RdsCpuAlarm', {
        alarmName: `${config.envName}-rds-cpu-high`,
        alarmDescription:
          'RDS CPU utilization sustained above threshold — may need scaling or query optimisation.',
        metric: rdsInstance.metricCPUUtilization({
          period: cdk.Duration.minutes(1),
          statistic: cloudwatch.Stats.AVERAGE,
        }),
        threshold: config.rdsCpuThreshold,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        evaluationPeriods: 5,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      })
    );

    // ---------------------
    // Alarm 4: RDS free storage
    // FreeStorageSpace is in bytes. Convert pct threshold to bytes using
    // dbAllocatedStorage (GiB).
    // ---------------------
    const allocatedBytes = config.dbAllocatedStorage * 1024 * 1024 * 1024;
    const freeStorageThresholdBytes =
      (config.rdsStorageThresholdPct / 100) * allocatedBytes;

    withActions(
      new cloudwatch.Alarm(this, 'RdsFreeStorageAlarm', {
        alarmName: `${config.envName}-rds-low-storage`,
        alarmDescription:
          'RDS free storage below threshold — risk of disk full.',
        metric: rdsInstance.metricFreeStorageSpace({
          period: cdk.Duration.minutes(1),
          statistic: cloudwatch.Stats.MINIMUM,
        }),
        threshold: freeStorageThresholdBytes,
        comparisonOperator: cloudwatch.ComparisonOperator.LESS_THAN_THRESHOLD,
        evaluationPeriods: 1,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      })
    );

    // ---------------------
    // Alarm 5: DLQ depth
    // Any message landing in the DLQ means a ledger permanently failed processing.
    // ---------------------
    withActions(
      new cloudwatch.Alarm(this, 'DlqDepthAlarm', {
        alarmName: `${config.envName}-ledger-processor-dlq-depth`,
        alarmDescription:
          'Ledger Processor DLQ has messages — one or more ledgers permanently failed processing.',
        metric: new cloudwatch.Metric({
          namespace: 'AWS/SQS',
          metricName: 'ApproximateNumberOfMessagesVisible',
          dimensionsMap: { QueueName: deadLetterQueue.queueName },
          period: cdk.Duration.minutes(1),
          statistic: cloudwatch.Stats.MAXIMUM,
          label: 'DLQ depth',
        }),
        threshold: 0,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        evaluationPeriods: 1,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      })
    );

    // ---------------------
    // Alarm 5b: Type-1 enrichment DLQ depth (task 0191)
    // Any message landing in the enrichment DLQ means an asset
    // permanently failed enrichment after maxReceiveCount=3 retries.
    // Same shape as Alarm 5.
    // ---------------------
    withActions(
      new cloudwatch.Alarm(this, 'EnrichmentDlqDepthAlarm', {
        alarmName: `${config.envName}-enrichment-dlq-depth`,
        alarmDescription:
          'Enrichment worker DLQ has messages — one or more assets permanently failed enrichment.',
        metric: new cloudwatch.Metric({
          namespace: 'AWS/SQS',
          metricName: 'ApproximateNumberOfMessagesVisible',
          dimensionsMap: { QueueName: enrichmentDlq.queueName },
          period: cdk.Duration.minutes(1),
          statistic: cloudwatch.Stats.MAXIMUM,
          label: 'Enrichment DLQ depth',
        }),
        threshold: 0,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        evaluationPeriods: 1,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      })
    );

    // ---------------------
    // Alarm 5c: Type-1 enrichment worker error rate (task 0191)
    // Mirrors Alarm 2 (ProcessorErrorRateAlarm) but for the worker
    // Lambda. Uses the same threshold config — both Lambdas have
    // similar acceptable error rates.
    // ---------------------
    const workerErrors = enrichmentWorkerFunction.metricErrors({
      period: cdk.Duration.minutes(5),
      statistic: cloudwatch.Stats.SUM,
    });
    const workerInvocations = enrichmentWorkerFunction.metricInvocations({
      period: cdk.Duration.minutes(5),
      statistic: cloudwatch.Stats.SUM,
    });
    withActions(
      new cloudwatch.Alarm(this, 'EnrichmentWorkerErrorRateAlarm', {
        alarmName: `${config.envName}-enrichment-worker-error-rate`,
        alarmDescription:
          'Enrichment worker Lambda error rate exceeded threshold — DB / network / SEP-1 issues.',
        metric: new cloudwatch.MathExpression({
          expression: 'errors / invocations',
          usingMetrics: {
            errors: workerErrors,
            invocations: workerInvocations,
          },
          period: cdk.Duration.minutes(5),
          label: 'Worker Error Rate',
        }),
        threshold: config.processorErrorRateThreshold,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        evaluationPeriods: 1,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      })
    );

    // ---------------------
    // Alarm 6: API Gateway 5xx rate
    // 5xxError / Count > threshold over 5-minute window.
    // ---------------------
    const stageName = restApi.deploymentStage.stageName;
    const apiName = restApi.restApiName;

    const api5xx = new cloudwatch.Metric({
      namespace: 'AWS/ApiGateway',
      metricName: '5XXError',
      dimensionsMap: { ApiName: apiName, Stage: stageName },
      period: cdk.Duration.minutes(5),
      statistic: cloudwatch.Stats.SUM,
    });
    const apiCount = new cloudwatch.Metric({
      namespace: 'AWS/ApiGateway',
      metricName: 'Count',
      dimensionsMap: { ApiName: apiName, Stage: stageName },
      period: cdk.Duration.minutes(5),
      statistic: cloudwatch.Stats.SUM,
    });

    withActions(
      new cloudwatch.Alarm(this, 'ApiGateway5xxAlarm', {
        alarmName: `${config.envName}-api-gateway-5xx-rate`,
        alarmDescription:
          'API Gateway 5xx error rate exceeded threshold — user-facing errors.',
        metric: new cloudwatch.MathExpression({
          expression: '(m5xx / mcount) * 100',
          usingMetrics: { m5xx: api5xx, mcount: apiCount },
          period: cdk.Duration.minutes(5),
          label: '5xx Rate (%)',
        }),
        threshold: config.apiGateway5xxThreshold,
        comparisonOperator:
          cloudwatch.ComparisonOperator.GREATER_THAN_THRESHOLD,
        evaluationPeriods: 1,
        treatMissingData: cloudwatch.TreatMissingData.NOT_BREACHING,
      })
    );

    // ---------------------
    // Dashboard
    // ---------------------
    new cloudwatch.Dashboard(this, 'Dashboard', {
      dashboardName: `${config.envName}-soroban-explorer`,
      widgets: [
        // Row 1: Ingestion section header
        [
          new cloudwatch.TextWidget({
            markdown: '## Ingestion',
            width: 24,
            height: 1,
          }),
        ],
        // Row 2: Galexie freshness proxy + last indexed ledger + Processor duration
        [
          new cloudwatch.GraphWidget({
            title: 'Galexie S3 freshness (Processor invocations/min)',
            left: [
              processorFunction.metricInvocations({
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.SUM,
                label: 'Invocations',
              }),
            ],
            width: 8,
            height: 6,
          }),
          new cloudwatch.GraphWidget({
            title: 'Last processed ledger sequence',
            left: [
              new cloudwatch.Metric({
                namespace: 'SorobanBlockExplorer/Indexer',
                metricName: 'LastProcessedLedgerSequence',
                dimensionsMap: { Environment: config.envName },
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.MAXIMUM,
                label: 'Last indexed ledger',
              }),
            ],
            width: 8,
            height: 6,
          }),
          new cloudwatch.GraphWidget({
            title: 'Ledger Processor duration (p50/p95/p99)',
            left: [
              processorFunction.metric('Duration', {
                period: cdk.Duration.minutes(5),
                statistic: 'p50',
                label: 'p50',
              }),
              processorFunction.metric('Duration', {
                period: cdk.Duration.minutes(5),
                statistic: 'p95',
                label: 'p95',
              }),
              processorFunction.metric('Duration', {
                period: cdk.Duration.minutes(5),
                statistic: 'p99',
                label: 'p99',
              }),
            ],
            width: 8,
            height: 6,
          }),
        ],
        // Row 3: Processor errors + DLQ depth
        [
          new cloudwatch.GraphWidget({
            title: 'Ledger Processor errors',
            left: [
              processorFunction.metricErrors({
                period: cdk.Duration.minutes(5),
                statistic: cloudwatch.Stats.SUM,
                label: 'Errors',
              }),
            ],
            width: 6,
            height: 6,
          }),
          new cloudwatch.GraphWidget({
            title: 'Ledger Processor DLQ depth',
            left: [
              new cloudwatch.Metric({
                namespace: 'AWS/SQS',
                metricName: 'ApproximateNumberOfMessagesVisible',
                dimensionsMap: { QueueName: deadLetterQueue.queueName },
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.MAXIMUM,
                label: 'DLQ depth',
              }),
            ],
            width: 6,
            height: 6,
          }),
          new cloudwatch.GraphWidget({
            title: 'Enrichment DLQ depth',
            left: [
              new cloudwatch.Metric({
                namespace: 'AWS/SQS',
                metricName: 'ApproximateNumberOfMessagesVisible',
                dimensionsMap: { QueueName: enrichmentDlq.queueName },
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.MAXIMUM,
                label: 'Enrichment DLQ depth',
              }),
            ],
            width: 6,
            height: 6,
          }),
          new cloudwatch.GraphWidget({
            title: 'Lambda concurrent executions',
            left: [
              new cloudwatch.Metric({
                namespace: 'AWS/Lambda',
                metricName: 'ConcurrentExecutions',
                dimensionsMap: {
                  FunctionName: processorFunction.functionName,
                },
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.MAXIMUM,
                label: 'Processor',
              }),
              new cloudwatch.Metric({
                namespace: 'AWS/Lambda',
                metricName: 'ConcurrentExecutions',
                dimensionsMap: { FunctionName: apiFunction.functionName },
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.MAXIMUM,
                label: 'API',
              }),
            ],
            width: 6,
            height: 6,
          }),
        ],
        // Row 4: API section header
        [
          new cloudwatch.TextWidget({
            markdown: '## API',
            width: 24,
            height: 1,
          }),
        ],
        // Row 5: API latency + 4xx/5xx
        [
          new cloudwatch.GraphWidget({
            title: 'API Lambda latency (p50/p95/p99)',
            left: [
              apiFunction.metric('Duration', {
                period: cdk.Duration.minutes(5),
                statistic: 'p50',
                label: 'p50',
              }),
              apiFunction.metric('Duration', {
                period: cdk.Duration.minutes(5),
                statistic: 'p95',
                label: 'p95',
              }),
              apiFunction.metric('Duration', {
                period: cdk.Duration.minutes(5),
                statistic: 'p99',
                label: 'p99',
              }),
            ],
            width: 12,
            height: 6,
          }),
          new cloudwatch.GraphWidget({
            title: 'API Gateway 4xx / 5xx errors',
            left: [
              new cloudwatch.Metric({
                namespace: 'AWS/ApiGateway',
                metricName: '4XXError',
                dimensionsMap: { ApiName: apiName, Stage: stageName },
                period: cdk.Duration.minutes(5),
                statistic: cloudwatch.Stats.SUM,
                label: '4xx',
              }),
              new cloudwatch.Metric({
                namespace: 'AWS/ApiGateway',
                metricName: '5XXError',
                dimensionsMap: { ApiName: apiName, Stage: stageName },
                period: cdk.Duration.minutes(5),
                statistic: cloudwatch.Stats.SUM,
                label: '5xx',
              }),
            ],
            width: 12,
            height: 6,
          }),
        ],
        // Row 6: API Gateway cache hit rate
        [
          new cloudwatch.GraphWidget({
            title: 'API Gateway cache hit / miss',
            left: [
              new cloudwatch.Metric({
                namespace: 'AWS/ApiGateway',
                metricName: 'CacheHitCount',
                dimensionsMap: { ApiName: apiName, Stage: stageName },
                period: cdk.Duration.minutes(5),
                statistic: cloudwatch.Stats.SUM,
                label: 'Cache hits',
              }),
              new cloudwatch.Metric({
                namespace: 'AWS/ApiGateway',
                metricName: 'CacheMissCount',
                dimensionsMap: { ApiName: apiName, Stage: stageName },
                period: cdk.Duration.minutes(5),
                statistic: cloudwatch.Stats.SUM,
                label: 'Cache misses',
              }),
            ],
            width: 12,
            height: 6,
          }),
          new cloudwatch.GraphWidget({
            title: 'Lambda cold starts',
            left: [
              processorFunction.metric('InitDuration', {
                period: cdk.Duration.minutes(5),
                statistic: cloudwatch.Stats.SAMPLE_COUNT,
                label: 'Processor cold starts',
              }),
              apiFunction.metric('InitDuration', {
                period: cdk.Duration.minutes(5),
                statistic: cloudwatch.Stats.SAMPLE_COUNT,
                label: 'API cold starts',
              }),
            ],
            width: 12,
            height: 6,
          }),
        ],
        // Row 7: Resources section header
        [
          new cloudwatch.TextWidget({
            markdown: '## Resources',
            width: 24,
            height: 1,
          }),
        ],
        // Row 8: RDS CPU + connections
        [
          new cloudwatch.GraphWidget({
            title: 'RDS CPU utilization',
            left: [
              rdsInstance.metricCPUUtilization({
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.AVERAGE,
                label: 'CPU %',
              }),
            ],
            leftAnnotations: [
              {
                value: config.rdsCpuThreshold,
                label: 'Alarm threshold',
                color: cloudwatch.Color.RED,
              },
            ],
            width: 12,
            height: 6,
          }),
          new cloudwatch.GraphWidget({
            title: 'RDS connections',
            left: [
              rdsInstance.metricDatabaseConnections({
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.AVERAGE,
                label: 'Connections',
              }),
            ],
            width: 12,
            height: 6,
          }),
        ],
        // Row 9: RDS free storage
        [
          new cloudwatch.GraphWidget({
            title: 'RDS free storage (bytes)',
            left: [
              rdsInstance.metricFreeStorageSpace({
                period: cdk.Duration.minutes(1),
                statistic: cloudwatch.Stats.MINIMUM,
                label: 'Free storage',
              }),
            ],
            leftAnnotations: [
              {
                value: freeStorageThresholdBytes,
                label: 'Alarm threshold',
                color: cloudwatch.Color.RED,
              },
            ],
            width: 12,
            height: 6,
          }),
        ],
      ],
    });

    // ---------------------
    // Tags
    // ---------------------
    cdk.Tags.of(this).add('Project', 'soroban-block-explorer');
    cdk.Tags.of(this).add('Environment', config.envName);
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
  }
}
