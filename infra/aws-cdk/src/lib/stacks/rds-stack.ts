import * as cdk from 'aws-cdk-lib';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as kms from 'aws-cdk-lib/aws-kms';
import * as rds from 'aws-cdk-lib/aws-rds';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

import { POSTGRESQL_PORT } from '../ports.js';

const SECRET_ROTATION_DAYS = 30;

export interface RdsStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  readonly vpc: ec2.IVpc;
  readonly lambdaSecurityGroup: ec2.ISecurityGroup;
  readonly ecsSecurityGroup: ec2.ISecurityGroup;
}

/**
 * RDS PostgreSQL with RDS Proxy and Secrets Manager.
 *
 * All Lambda connections go through RDS Proxy (never direct).
 * Proxy handles credential rotation transparently.
 *
 * No read replica at launch. Add one when CPU exceeds monitoring threshold.
 */
export class RdsStack extends cdk.Stack {
  readonly dbProxy?: rds.IDatabaseProxy;
  readonly dbInstance: rds.IDatabaseInstance;
  readonly dbSecret: secretsmanager.ISecret;
  readonly rdsSecurityGroup: ec2.ISecurityGroup;

  constructor(scope: Construct, id: string, props: RdsStackProps) {
    super(scope, id, props);

    const { config, vpc, lambdaSecurityGroup, ecsSecurityGroup } = props;
    const prefix = config.envName;

    // ---------------------
    // Security Group
    // ---------------------
    // Allows inbound from Lambda and ECS on PostgreSQL port.
    const rdsSg = new ec2.SecurityGroup(this, 'RdsSg', {
      vpc,
      description: 'RDS PostgreSQL and RDS Proxy',
      allowAllOutbound: false,
    });
    rdsSg.addIngressRule(
      lambdaSecurityGroup,
      ec2.Port.tcp(POSTGRESQL_PORT),
      'Allow Lambda to RDS on PostgreSQL port'
    );
    rdsSg.addIngressRule(
      ecsSecurityGroup,
      ec2.Port.tcp(POSTGRESQL_PORT),
      'Allow ECS to RDS on PostgreSQL port'
    );
    // Self-referencing: Proxy and rotation Lambda (same SG) need to reach RDS
    rdsSg.addIngressRule(
      rdsSg,
      ec2.Port.tcp(POSTGRESQL_PORT),
      'Allow Proxy/rotation Lambda to RDS (self-reference)'
    );
    this.rdsSecurityGroup = rdsSg;

    // ---------------------
    // KMS Key (production only)
    // ---------------------
    const encryptionKey = config.kmsEncryption
      ? new kms.Key(this, 'EncryptionKey', {
          alias: `${prefix}-soroban-explorer-rds`,
          description: 'Encryption key for RDS (Soroban Block Explorer)',
          enableKeyRotation: true,
          removalPolicy: cdk.RemovalPolicy.RETAIN,
        })
      : undefined;

    // ---------------------
    // Secrets Manager
    // ---------------------
    // CDK auto-generates the password. No credentials in code or config.
    const dbSecret = new rds.DatabaseSecret(this, 'DbSecret', {
      username: 'explorer',
      secretName: `soroban-explorer/${prefix}/rds-credentials`,
      excludeCharacters: '"@/\\',
    });
    this.dbSecret = dbSecret;

    // ---------------------
    // RDS PostgreSQL
    // ---------------------
    const dbInstance = new rds.DatabaseInstance(this, 'Postgres', {
      engine: rds.DatabaseInstanceEngine.postgres({
        version: rds.PostgresEngineVersion.VER_16_4,
      }),
      instanceType: new ec2.InstanceType(config.dbInstanceClass),
      credentials: rds.Credentials.fromSecret(dbSecret),
      databaseName: 'soroban_explorer',
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
      securityGroups: [rdsSg],
      multiAz: config.dbMultiAz,
      allocatedStorage: config.dbAllocatedStorage,
      storageType: rds.StorageType.GP3,
      storageEncrypted: true,
      storageEncryptionKey: encryptionKey,
      publiclyAccessible: false,
      deletionProtection: config.dbDeletionProtection,
      backupRetention: cdk.Duration.days(config.dbBackupRetentionDays),
      removalPolicy: config.dbDeletionProtection
        ? cdk.RemovalPolicy.RETAIN
        : cdk.RemovalPolicy.SNAPSHOT,
      // TLS enforced on production (kmsEncryption = true), optional on staging
      ...(config.kmsEncryption && {
        parameterGroup: new rds.ParameterGroup(this, 'PgParams', {
          engine: rds.DatabaseInstanceEngine.postgres({
            version: rds.PostgresEngineVersion.VER_16_4,
          }),
          parameters: {
            'rds.force_ssl': '1',
          },
        }),
      }),
    });
    this.dbInstance = dbInstance;

    // ---------------------
    // RDS Proxy (optional per config)
    // ---------------------
    // Required for Lambda burst traffic (backfill). Can be disabled
    // to save ~$20/mo when not needed.
    if (config.dbProxy) {
      const proxy = new rds.DatabaseProxy(this, 'Proxy', {
        proxyTarget: rds.ProxyTarget.fromInstance(dbInstance),
        secrets: [dbSecret],
        vpc,
        vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
        securityGroups: [rdsSg],
        requireTLS: true,
        dbProxyName: `${prefix}-soroban-explorer`,
      });
      this.dbProxy = proxy;

      new cdk.CfnOutput(this, 'DbProxyEndpoint', {
        value: proxy.endpoint,
      });
    }

    // ---------------------
    // Secret rotation (production: 30-day cycle)
    // ---------------------
    if (config.kmsEncryption) {
      dbSecret.addRotationSchedule('Rotation', {
        automaticallyAfter: cdk.Duration.days(SECRET_ROTATION_DAYS),
        hostedRotation: secretsmanager.HostedRotation.postgreSqlSingleUser({
          vpc,
          vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
          securityGroups: [rdsSg],
        }),
      });
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
    new cdk.CfnOutput(this, 'DbEndpoint', {
      value: dbInstance.instanceEndpoint.hostname,
    });
    new cdk.CfnOutput(this, 'DbSecretArn', {
      value: dbSecret.secretArn,
    });
  }
}
