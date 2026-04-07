import * as cdk from 'aws-cdk-lib';
import * as iam from 'aws-cdk-lib/aws-iam';
import type { Construct } from 'constructs';

const GITHUB_OIDC_THUMBPRINT = 'ffffffffffffffffffffffffffffffffffffffff';
const GITHUB_OIDC_ISSUER = 'https://token.actions.githubusercontent.com';
const GITHUB_OIDC_AUDIENCE = 'sts.amazonaws.com';

export interface CicdStackProps extends cdk.StackProps {
  /** GitHub org/repo, e.g. "rumblefishdev/soroban-block-explorer" */
  readonly githubRepo: string;
  readonly awsRegion: string;
}

/**
 * CI/CD resources shared across environments.
 *
 * Creates:
 * - GitHub Actions OIDC identity provider (singleton per AWS account)
 * - Staging deploy role (scoped to GitHub Environment "staging")
 * - Production deploy role (scoped to GitHub Environment "production")
 *
 * Deploy roles trust the CDK bootstrap roles for actual CloudFormation
 * operations. The OIDC trust policy restricts which GitHub workflows
 * can assume each role based on the GitHub Environment name.
 *
 * Deployed once per AWS account via: `npx cdk --app "node dist/bin/cicd.js" deploy`
 */
export class CicdStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: CicdStackProps) {
    super(scope, id, props);

    const { githubRepo, awsRegion } = props;
    const accountId = cdk.Stack.of(this).account;

    // ---------------------
    // GitHub Actions OIDC Provider
    // ---------------------
    // Singleton per AWS account. GitHub's OIDC thumbprint is not used
    // for validation (GitHub uses a well-known JWKS endpoint), but CDK
    // requires at least one thumbprint. Use the conventional placeholder.
    const oidcProvider = new iam.OpenIdConnectProvider(
      this,
      'GitHubOidcProvider',
      {
        url: GITHUB_OIDC_ISSUER,
        clientIds: [GITHUB_OIDC_AUDIENCE],
        thumbprints: [GITHUB_OIDC_THUMBPRINT],
      }
    );

    // ---------------------
    // Deploy Roles
    // ---------------------
    // Each role trusts GitHub Actions OIDC with an environment condition.
    // The role can then assume CDK bootstrap roles to perform CloudFormation
    // operations. No direct CloudFormation/S3/IAM permissions needed —
    // CDK bootstrapped deploys use the bootstrap execution role.
    for (const envName of ['staging', 'production'] as const) {
      const role = new iam.Role(this, `${capitalize(envName)}DeployRole`, {
        roleName: `soroban-explorer-${envName}-deploy`,
        assumedBy: new iam.WebIdentityPrincipal(
          oidcProvider.openIdConnectProviderArn,
          {
            StringEquals: {
              [`${GITHUB_OIDC_ISSUER}:aud`]: GITHUB_OIDC_AUDIENCE,
              [`${GITHUB_OIDC_ISSUER}:sub`]: `repo:${githubRepo}:environment:${envName}`,
            },
          }
        ),
        maxSessionDuration: cdk.Duration.hours(1),
        description: `GitHub Actions deploy role for ${envName} environment`,
      });

      // Allow assuming CDK bootstrap roles for CloudFormation operations.
      // CDK bootstrap creates roles with a well-known naming pattern.
      role.addToPolicy(
        new iam.PolicyStatement({
          actions: ['sts:AssumeRole'],
          resources: [
            `arn:aws:iam::${accountId}:role/cdk-hnb659fds-*-${accountId}-${awsRegion}`,
          ],
        })
      );

      // ECR login + push for Galexie image mirroring.
      // Scoped to the environment's ECR repo ARN.
      role.addToPolicy(
        new iam.PolicyStatement({
          actions: [
            'ecr:BatchCheckLayerAvailability',
            'ecr:GetDownloadUrlForLayer',
            'ecr:BatchGetImage',
            'ecr:PutImage',
            'ecr:InitiateLayerUpload',
            'ecr:UploadLayerPart',
            'ecr:CompleteLayerUpload',
          ],
          resources: [
            `arn:aws:ecr:${awsRegion}:${accountId}:repository/${envName}-galexie`,
          ],
        })
      );

      // ECR GetAuthorizationToken doesn't support resource restrictions.
      role.addToPolicy(
        new iam.PolicyStatement({
          actions: ['ecr:GetAuthorizationToken'],
          resources: ['*'],
        })
      );

      // SSM read for ECR repo URI lookup.
      role.addToPolicy(
        new iam.PolicyStatement({
          actions: ['ssm:GetParameter'],
          resources: [
            `arn:aws:ssm:${awsRegion}:${accountId}:parameter/soroban-explorer/${envName}/*`,
          ],
        })
      );

      // CloudFormation read for post-deploy smoke test (describe stack outputs).
      role.addToPolicy(
        new iam.PolicyStatement({
          actions: ['cloudformation:DescribeStacks'],
          resources: [
            `arn:aws:cloudformation:${awsRegion}:${accountId}:stack/Explorer-${envName}-*/*`,
          ],
        })
      );

      // Output the role ARN — store as GitHub Environment secret.
      new cdk.CfnOutput(this, `${capitalize(envName)}DeployRoleArn`, {
        value: role.roleArn,
        description: `Deploy role ARN for ${envName} — add as AWS_DEPLOY_ROLE_ARN in GitHub Environment "${envName}"`,
      });
    }

    // ---------------------
    // Tags
    // ---------------------
    cdk.Tags.of(this).add('Project', 'soroban-block-explorer');
    cdk.Tags.of(this).add('ManagedBy', 'cdk');
  }
}

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}
