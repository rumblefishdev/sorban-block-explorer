import * as cdk from 'aws-cdk-lib';
import * as cr from 'aws-cdk-lib/custom-resources';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as secretsmanager from 'aws-cdk-lib/aws-secretsmanager';
import { RustFunction } from 'cargo-lambda-cdk';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

export interface MigrationStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
  readonly vpc: ec2.IVpc;
  readonly lambdaSecurityGroup: ec2.ISecurityGroup;
  readonly dbSecret: secretsmanager.ISecret;
  readonly dbProxyEndpoint: string;
  readonly cargoWorkspacePath: string;
}

/**
 * Database migration stack for the Soroban Block Explorer.
 *
 * Runs sqlx migrations as a CloudFormation custom resource before
 * application Lambdas are deployed. Migration failure blocks deployment.
 *
 * Dependency ordering (enforced in app.ts):
 *   NetworkStack → RdsStack → MigrationStack → ComputeStack
 */
export class MigrationStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: MigrationStackProps) {
    super(scope, id, props);

    const {
      config,
      vpc,
      lambdaSecurityGroup,
      dbSecret,
      dbProxyEndpoint,
      cargoWorkspacePath,
    } = props;

    // ---------------------
    // Migration Lambda
    // ---------------------
    // Built from crates/db-migrate. Resolves credentials from Secrets
    // Manager, connects through RDS Proxy, and runs pending migrations.
    const migrationFn = new RustFunction(this, 'MigrationFunction', {
      functionName: `${config.envName}-soroban-explorer-db-migrate`,
      manifestPath: cargoWorkspacePath,
      binaryName: 'db-migrate',
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

    dbSecret.grantRead(migrationFn);

    // ---------------------
    // CDK Provider + Custom Resource
    // ---------------------
    // Provider wraps the Lambda as a CloudFormation custom resource handler.
    // On Create/Update it invokes the Lambda; if it fails, CloudFormation
    // rolls back the stack and ComputeStack is never updated.
    const provider = new cr.Provider(this, 'MigrationProvider', {
      onEventHandler: migrationFn,
      logRetention: logs.RetentionDays.ONE_MONTH,
    });

    // migrationVersion uses Date.now() to force a re-invocation on every
    // deployment. This is safe because sqlx migrations are idempotent —
    // already-applied migrations are skipped.
    new cdk.CustomResource(this, 'RunMigrations', {
      serviceToken: provider.serviceToken,
      properties: {
        migrationVersion: Date.now().toString(),
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
