import * as cdk from 'aws-cdk-lib';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as iam from 'aws-cdk-lib/aws-iam';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../types.js';

import { HTTPS_PORT, POSTGRESQL_PORT, STELLAR_OVERLAY_PORT } from '../ports.js';

export interface NetworkStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * Foundational networking layer for the Soroban Block Explorer.
 *
 * Creates a VPC with public/private subnet split in a single AZ (us-east-1a),
 * security groups for compute components, and an S3 Gateway VPC endpoint.
 *
 * RDS security group lives in RdsStack to avoid cross-stack cyclic
 * references. Lambda/ECS egress to RDS on port 5432 uses VPC CIDR
 * (configured here) instead of SG-to-SG reference.
 *
 * Multi-AZ expansion: add AZ entries to `config.availabilityZones`.
 * CDK provisions new subnets and NAT Gateways automatically.
 */
export class NetworkStack extends cdk.Stack {
  readonly vpc: ec2.IVpc;
  readonly lambdaSecurityGroup: ec2.ISecurityGroup;
  readonly ecsSecurityGroup: ec2.ISecurityGroup;

  constructor(scope: Construct, id: string, props: NetworkStackProps) {
    super(scope, id, props);

    const { config } = props;

    // ---------------------
    // VPC
    // ---------------------
    // Two AZs required by RDS subnet group (AWS minimum).
    // NAT Gateway count configurable — increase to one per AZ for HA when SLA > 99.9%.
    const natProvider =
      config.natType === 'instance'
        ? ec2.NatProvider.instanceV2({
            instanceType: new ec2.InstanceType('t3.micro'),
          })
        : ec2.NatProvider.gateway();

    const vpc = new ec2.Vpc(this, 'Vpc', {
      ipAddresses: ec2.IpAddresses.cidr(config.vpcCidr),
      availabilityZones: [...config.availabilityZones],
      natGateways: config.natGatewayCount,
      natGatewayProvider: natProvider,
      restrictDefaultSecurityGroup: true,
      subnetConfiguration: [
        {
          name: 'Public',
          subnetType: ec2.SubnetType.PUBLIC,
          cidrMask: 20,
        },
        {
          name: 'Private',
          subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS,
          cidrMask: 20,
        },
      ],
    });
    this.vpc = vpc;

    // ---------------------
    // Security Groups
    // ---------------------
    // RDS SG and DB-related egress rules are in RdsStack to avoid
    // cross-stack cyclic references between Network and Storage.

    // Lambda — API handler, Ledger Processor
    const lambdaSg = new ec2.SecurityGroup(this, 'LambdaSg', {
      vpc,
      description:
        'Lambda functions (API, Ledger Processor, Event Interpreter)',
      allowAllOutbound: false,
    });
    // Outbound HTTPS to 0.0.0.0/0 — intentionally broad. Lambda needs access to
    // multiple AWS APIs (Secrets Manager, CloudWatch, X-Ray, STS). VPC Interface
    // Endpoints (~$7/mo each) are not cost-justified at launch. S3 traffic is
    // routed via the free Gateway endpoint at route-table level.
    // Outbound to RDS/RDS Proxy on PostgreSQL port. Uses VPC CIDR instead of
    // SG-to-SG reference to avoid cross-stack cyclic dependency with RdsStack.
    // Only RDS listens on 5432 within the VPC.
    lambdaSg.addEgressRule(
      ec2.Peer.ipv4(config.vpcCidr),
      ec2.Port.tcp(POSTGRESQL_PORT),
      'Allow Lambda to RDS/Proxy on PostgreSQL port (VPC scope)'
    );
    lambdaSg.addEgressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(HTTPS_PORT),
      'Allow Lambda to HTTPS (AWS APIs, S3 via VPC endpoint)'
    );

    // ECS — Fargate tasks (Galexie live + backfill)
    const ecsSg = new ec2.SecurityGroup(this, 'EcsSg', {
      vpc,
      description: 'ECS Fargate tasks (Galexie ingestion)',
      allowAllOutbound: false,
    });
    // Outbound HTTPS to 0.0.0.0/0 — intentionally broad. ECS needs ECR image
    // pull, CloudWatch Logs, and Stellar history archive access. VPC Interface
    // Endpoints are not cost-justified at launch. S3 via free Gateway endpoint.
    // Outbound to RDS on PostgreSQL port. VPC CIDR scope (see Lambda SG comment).
    ecsSg.addEgressRule(
      ec2.Peer.ipv4(config.vpcCidr),
      ec2.Port.tcp(POSTGRESQL_PORT),
      'Allow ECS to RDS on PostgreSQL port (VPC scope)'
    );
    ecsSg.addEgressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(HTTPS_PORT),
      'Allow ECS to HTTPS (ECR, CloudWatch, S3 via VPC endpoint)'
    );
    // Outbound for Stellar network peer connections.
    // Galexie connects to Stellar peers on port 11625 (overlay protocol).
    // History archives use HTTPS (covered by the 443 rule above).
    ecsSg.addEgressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(STELLAR_OVERLAY_PORT),
      'Allow ECS to Stellar peer network (overlay protocol)'
    );

    this.lambdaSecurityGroup = lambdaSg;
    this.ecsSecurityGroup = ecsSg;

    // ---------------------
    // S3 Gateway VPC Endpoint
    // ---------------------
    // Gateway type (free, no hourly cost). Adds route table entries so
    // S3 traffic from private subnets stays within AWS network instead of
    // traversing the NAT Gateway — reduces cost and improves latency.
    //
    // Endpoint policy restricts access to the project's ledger data bucket
    // and CDK staging buckets (defense in depth — IAM still applies).
    const ledgerBucketName = `${config.envName}-stellar-ledger-data`;
    const s3Endpoint = vpc.addGatewayEndpoint('S3Endpoint', {
      service: ec2.GatewayVpcEndpointAwsService.S3,
      subnets: [{ subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS }],
    });
    // Allow all S3 actions on our bucket — IAM roles provide the
    // fine-grained action-level control (grantWrite, grantRead).
    // Endpoint policy restricts WHICH buckets, IAM restricts WHICH actions.
    s3Endpoint.addToPolicy(
      new iam.PolicyStatement({
        principals: [new iam.AnyPrincipal()],
        actions: ['s3:*'],
        resources: [
          `arn:aws:s3:::${ledgerBucketName}`,
          `arn:aws:s3:::${ledgerBucketName}/*`,
        ],
      })
    );
    // Allow CDK bootstrap bucket — required for cdk deploy to upload
    // Lambda bundles and CloudFormation templates. The CDK bootstrap
    // bucket follows the pattern: cdk-hnb659fds-assets-{ACCOUNT}-{REGION}.
    // Scoped to this fixed prefix to avoid matching arbitrary third-party
    // buckets that happen to start with "cdk-".
    s3Endpoint.addToPolicy(
      new iam.PolicyStatement({
        principals: [new iam.AnyPrincipal()],
        actions: ['s3:*'],
        resources: [
          'arn:aws:s3:::cdk-hnb659fds-*',
          'arn:aws:s3:::cdk-hnb659fds-*/*',
        ],
      })
    );

    // ---------------------
    // Tags
    // ---------------------
    cdk.Tags.of(this).add('Project', 'soroban-block-explorer');
    cdk.Tags.of(this).add('Environment', config.envName);
    cdk.Tags.of(this).add('ManagedBy', 'cdk');

    // ---------------------
    // Outputs
    // ---------------------
    new cdk.CfnOutput(this, 'VpcId', { value: vpc.vpcId });
    new cdk.CfnOutput(this, 'PrivateSubnetIds', {
      value: vpc
        .selectSubnets({ subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS })
        .subnetIds.join(','),
    });
    new cdk.CfnOutput(this, 'PublicSubnetIds', {
      value: vpc
        .selectSubnets({ subnetType: ec2.SubnetType.PUBLIC })
        .subnetIds.join(','),
    });
    new cdk.CfnOutput(this, 'LambdaSecurityGroupId', {
      value: lambdaSg.securityGroupId,
    });
    new cdk.CfnOutput(this, 'EcsSecurityGroupId', {
      value: ecsSg.securityGroupId,
    });
  }
}
