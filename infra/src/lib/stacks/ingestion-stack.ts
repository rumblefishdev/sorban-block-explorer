import * as cdk from 'aws-cdk-lib';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as ecr from 'aws-cdk-lib/aws-ecr';
import * as ecs from 'aws-cdk-lib/aws-ecs';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

const STOP_TIMEOUT_MIN = 2;
const STOP_TIMEOUT_MAX = 120;

/** Validate stopTimeout is within ECS allowed range (2–120 seconds). */
function validateStopTimeout(seconds: number): void {
  if (seconds < STOP_TIMEOUT_MIN || seconds > STOP_TIMEOUT_MAX) {
    throw new Error(
      `galexieStopTimeout must be between ${STOP_TIMEOUT_MIN} and ${STOP_TIMEOUT_MAX} seconds, got ${seconds}`
    );
  }
}

/** Map numeric retention days to the CDK enum. */
function toRetentionDays(days: number): logs.RetentionDays {
  const mapping: Record<number, logs.RetentionDays> = {
    1: logs.RetentionDays.ONE_DAY,
    3: logs.RetentionDays.THREE_DAYS,
    5: logs.RetentionDays.FIVE_DAYS,
    7: logs.RetentionDays.ONE_WEEK,
    14: logs.RetentionDays.TWO_WEEKS,
    30: logs.RetentionDays.ONE_MONTH,
    60: logs.RetentionDays.TWO_MONTHS,
    90: logs.RetentionDays.THREE_MONTHS,
    120: logs.RetentionDays.FOUR_MONTHS,
    150: logs.RetentionDays.FIVE_MONTHS,
    180: logs.RetentionDays.SIX_MONTHS,
    365: logs.RetentionDays.ONE_YEAR,
  };
  const result = mapping[days];
  if (result === undefined) {
    throw new Error(
      `Unsupported ecsLogRetentionDays value: ${days}. ` +
        `Supported: ${Object.keys(mapping).join(', ')}`
    );
  }
  return result;
}

export interface IngestionStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  readonly vpc: ec2.IVpc;
  readonly ecsSecurityGroup: ec2.ISecurityGroup;
  readonly ledgerBucketArn: string;
  readonly ledgerBucketName: string;
}

/**
 * ECS Fargate ingestion layer for the Soroban Block Explorer.
 *
 * Contains:
 * - ECR repository for Galexie container images
 * - ECS cluster with Container Insights
 * - Galexie live service (continuous ledger export)
 * - Galexie backfill task definition (on-demand historical import)
 * - IAM roles: task role (S3 write) + execution role (ECR pull)
 *
 * Both workloads write LedgerCloseMetaBatch XDR files to the
 * stellar-ledger-data S3 bucket. The S3 PutObject event triggers
 * the Ledger Processor Lambda (configured in ComputeStack).
 */
export class IngestionStack extends cdk.Stack {
  readonly cluster: ecs.ICluster;
  readonly repository: ecr.IRepository;
  readonly liveService: ecs.FargateService;
  readonly backfillTaskDefinition?: ecs.FargateTaskDefinition;

  constructor(scope: Construct, id: string, props: IngestionStackProps) {
    super(scope, id, props);

    const { config, vpc, ecsSecurityGroup, ledgerBucketArn, ledgerBucketName } =
      props;

    const logRetention = toRetentionDays(config.ecsLogRetentionDays);
    validateStopTimeout(config.galexieStopTimeout);

    // Import the ledger bucket by ARN/name to avoid cross-stack
    // cyclic dependency (same pattern as ComputeStack).
    const ledgerBucket = s3.Bucket.fromBucketAttributes(this, 'LedgerBucket', {
      bucketArn: ledgerBucketArn,
      bucketName: ledgerBucketName,
    });

    // ---------------------
    // ECR Repository
    // ---------------------
    const repository = new ecr.Repository(this, 'GalexieRepo', {
      repositoryName: `${config.envName}-galexie`,
      imageScanOnPush: true,
      encryption: config.kmsEncryption
        ? ecr.RepositoryEncryption.KMS
        : ecr.RepositoryEncryption.AES_256,
      removalPolicy: config.kmsEncryption
        ? cdk.RemovalPolicy.RETAIN
        : cdk.RemovalPolicy.DESTROY,
      emptyOnDelete: !config.kmsEncryption,
      lifecycleRules: [
        {
          description: 'Expire untagged images after 7 days',
          maxImageAge: cdk.Duration.days(7),
          rulePriority: 1,
          tagStatus: ecr.TagStatus.UNTAGGED,
        },
        {
          description: 'Retain last 10 images',
          maxImageCount: 10,
          rulePriority: 2,
          tagStatus: ecr.TagStatus.ANY,
        },
      ],
    });
    this.repository = repository;

    // Publish ECR repository URI to SSM so CI/CD (GitHub Actions) can
    // discover it without hardcoding the repository name.
    new ssm.StringParameter(this, 'EcrRepoUriParam', {
      parameterName: `/soroban-explorer/${config.envName}/ecr-galexie-repo-uri`,
      stringValue: repository.repositoryUri,
    });

    // ---------------------
    // ECS Cluster
    // ---------------------
    const cluster = new ecs.Cluster(this, 'IngestionCluster', {
      clusterName: `${config.envName}-ingestion`,
      vpc,
      containerInsightsV2: ecs.ContainerInsights.ENABLED,
    });
    this.cluster = cluster;

    // ---------------------
    // CloudWatch Log Groups
    // ---------------------
    const liveLogGroup = new logs.LogGroup(this, 'LiveLogGroup', {
      logGroupName: `/ecs/${config.envName}/galexie-live`,
      retention: logRetention,
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });

    const backfillLogGroup = new logs.LogGroup(this, 'BackfillLogGroup', {
      logGroupName: `/ecs/${config.envName}/galexie-backfill`,
      retention: logRetention,
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });

    // ---------------------
    // Shared container environment
    // ---------------------
    const sharedEnvironment: Record<string, string> = {
      NETWORK_PASSPHRASE: config.stellarNetworkPassphrase,
      DESTINATION: `s3://${ledgerBucket.bucketName}`,
      STELLAR_CORE_BINARY_PATH: '/usr/bin/stellar-core',
    };

    // Collect task definitions for shared IAM grants below.
    const taskDefs: ecs.FargateTaskDefinition[] = [];

    // ---------------------
    // Galexie Live — Task Definition
    // ---------------------
    const liveTaskDef = new ecs.FargateTaskDefinition(this, 'LiveTaskDef', {
      family: `${config.envName}-galexie-live`,
      cpu: config.galexieCpu,
      memoryLimitMiB: config.galexieMemory,
      ephemeralStorageGiB: config.galexieEphemeralStorage,
      runtimePlatform: {
        cpuArchitecture: ecs.CpuArchitecture.X86_64,
        operatingSystemFamily: ecs.OperatingSystemFamily.LINUX,
      },
    });

    // Writable volumes on ephemeral storage — required because
    // readonlyRootFilesystem is enabled. Captive Core needs /data
    // for ledger state and /tmp for general temp files.
    liveTaskDef.addVolume({ name: 'data' });
    liveTaskDef.addVolume({ name: 'tmp' });

    const liveContainer = liveTaskDef.addContainer('Galexie', {
      image: ecs.ContainerImage.fromEcrRepository(
        repository,
        config.galexieImageTag
      ),
      logging: ecs.LogDrivers.awsLogs({
        logGroup: liveLogGroup,
        streamPrefix: 'galexie',
      }),
      environment: {
        ...sharedEnvironment,
        // Append mode — Galexie auto-detects last exported ledger from S3.
        START: '',
      },
      // TODO: Add container health check once the image is running on staging.
      // Needs investigation into what's available inside stellar/stellar-galexie
      // (e.g. process liveness, last-file-age, HTTP endpoint). A naive pgrep
      // check only detects crashes, not hangs — a meaningful check should
      // verify that Galexie is actually producing data.
      readonlyRootFilesystem: true,
      stopTimeout: cdk.Duration.seconds(config.galexieStopTimeout),
    });
    liveContainer.addMountPoints(
      { containerPath: '/data', sourceVolume: 'data', readOnly: false },
      { containerPath: '/tmp', sourceVolume: 'tmp', readOnly: false }
    );
    taskDefs.push(liveTaskDef);

    // ---------------------
    // Galexie Live — Fargate Service
    // ---------------------
    const liveService = new ecs.FargateService(this, 'LiveService', {
      serviceName: `${config.envName}-galexie-live`,
      cluster,
      taskDefinition: liveTaskDef,
      desiredCount: config.galexieDesiredCount,
      securityGroups: [ecsSecurityGroup],
      vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
      assignPublicIp: false,
      platformVersion: ecs.FargatePlatformVersion.LATEST,
      // ECS Exec for debugging — staging only. For production, run a
      // one-off task definition update to enable when needed.
      enableExecuteCommand: config.ecsExecEnabled,
      circuitBreaker: { rollback: true },
      // Singleton service — allow full replacement (old task killed before
      // new one starts). Zero-downtime is not applicable: only one Galexie
      // writer can run at a time to avoid duplicate S3 writes.
      minHealthyPercent: 0,
      maxHealthyPercent: 100,
    });
    this.liveService = liveService;

    // ---------------------
    // Galexie Backfill — Task Definition (optional)
    // ---------------------
    // Uses the same Galexie image as the live service — Galexie handles
    // both append (live) and bounded-range (backfill) modes in the same
    // binary. The difference is the START/END environment variables.
    if (config.galexieBackfillEnabled) {
      const backfillTaskDef = new ecs.FargateTaskDefinition(
        this,
        'BackfillTaskDef',
        {
          family: `${config.envName}-galexie-backfill`,
          cpu: config.galexieCpu,
          memoryLimitMiB: config.galexieMemory,
          ephemeralStorageGiB: config.galexieEphemeralStorage,
          runtimePlatform: {
            cpuArchitecture: ecs.CpuArchitecture.X86_64,
            operatingSystemFamily: ecs.OperatingSystemFamily.LINUX,
          },
        }
      );

      backfillTaskDef.addVolume({ name: 'data' });
      backfillTaskDef.addVolume({ name: 'tmp' });

      const backfillContainer = backfillTaskDef.addContainer('Galexie', {
        image: ecs.ContainerImage.fromEcrRepository(
          repository,
          config.galexieImageTag
        ),
        logging: ecs.LogDrivers.awsLogs({
          logGroup: backfillLogGroup,
          streamPrefix: 'galexie',
        }),
        environment: {
          ...sharedEnvironment,
          // START and END are overridden per RunTask invocation.
          // Defaults here serve as documentation; actual values are
          // passed via container overrides when running the task.
          // Soroban mainnet activation ledger (Protocol 20, Feb 20 2024).
          START: '50457424',
          END: '',
        },
        readonlyRootFilesystem: true,
        stopTimeout: cdk.Duration.seconds(config.galexieStopTimeout),
      });
      backfillContainer.addMountPoints(
        { containerPath: '/data', sourceVolume: 'data', readOnly: false },
        { containerPath: '/tmp', sourceVolume: 'tmp', readOnly: false }
      );
      this.backfillTaskDefinition = backfillTaskDef;
      taskDefs.push(backfillTaskDef);
    }

    // ---------------------
    // IAM Grants
    // ---------------------
    // Task role — application permissions (S3 write + list for checkpoint resume).
    for (const taskDef of taskDefs) {
      ledgerBucket.grantWrite(taskDef.taskRole);
      // ListBucket is needed for Galexie checkpoint resume (scans S3 for
      // last exported ledger). IBucket.grant() is not available on imported
      // buckets, so we add the permission via inline policy.
      taskDef.taskRole.addToPrincipalPolicy(
        new iam.PolicyStatement({
          actions: ['s3:ListBucket'],
          resources: [ledgerBucket.bucketArn],
        })
      );
    }

    // Execution role — ECR pull (auto-granted by fromEcrRepository for image pull,
    // but explicit grantPull ensures GetAuthorizationToken is also granted).
    // FargateTaskDefinition always creates an execution role, so the non-null
    // assertion is safe here.
    for (const taskDef of taskDefs) {
      // eslint-disable-next-line @typescript-eslint/no-non-null-assertion
      repository.grantPull(taskDef.executionRole!);
    }

    // ECS Exec requires SSM permissions on the task role.
    // ssmmessages actions do not support resource-level restrictions (AWS limitation).
    if (config.ecsExecEnabled) {
      for (const taskDef of taskDefs) {
        taskDef.taskRole.addToPrincipalPolicy(
          new iam.PolicyStatement({
            actions: [
              'ssmmessages:CreateControlChannel',
              'ssmmessages:CreateDataChannel',
              'ssmmessages:OpenControlChannel',
              'ssmmessages:OpenDataChannel',
            ],
            resources: ['*'],
          })
        );
      }
    }

    // ---------------------
    // Tags
    // ---------------------
    cdk.Tags.of(this).add('Project', 'soroban-block-explorer');
    cdk.Tags.of(this).add('Environment', config.envName);
    cdk.Tags.of(this).add('ManagedBy', 'cdk');

    // ---------------------
    // Outputs
    // ---------------------
    new cdk.CfnOutput(this, 'ClusterName', {
      value: cluster.clusterName,
    });
    new cdk.CfnOutput(this, 'ClusterArn', {
      value: cluster.clusterArn,
    });
    new cdk.CfnOutput(this, 'RepositoryUri', {
      value: repository.repositoryUri,
    });
    new cdk.CfnOutput(this, 'LiveServiceName', {
      value: liveService.serviceName,
    });
    if (this.backfillTaskDefinition) {
      new cdk.CfnOutput(this, 'BackfillTaskDefArn', {
        value: this.backfillTaskDefinition.taskDefinitionArn,
      });
    }
  }
}
