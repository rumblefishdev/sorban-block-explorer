import * as cdk from 'aws-cdk-lib';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import type { Construct } from 'constructs';

import type { EnvironmentConfig } from '../config/types.js';

export interface NetworkStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * Foundational networking layer for the Soroban Block Explorer.
 *
 * Creates a VPC with public/private subnet split in a single AZ (us-east-1a),
 * security groups for inter-component access, and an S3 Gateway VPC endpoint.
 *
 * Multi-AZ expansion: add AZ entries to `config.availabilityZones`.
 * CDK provisions new subnets and NAT Gateways automatically.
 */
export class NetworkStack extends cdk.Stack {
  readonly vpc: ec2.IVpc;
  readonly lambdaSecurityGroup: ec2.ISecurityGroup;
  readonly rdsSecurityGroup: ec2.ISecurityGroup;
  readonly ecsSecurityGroup: ec2.ISecurityGroup;

  constructor(scope: Construct, id: string, props: NetworkStackProps) {
    super(scope, id, props);

    const { config } = props;

    // ---------------------
    // VPC
    // ---------------------
    // Single-AZ at launch (us-east-1a). One NAT Gateway in the public subnet
    // provides outbound internet for the private subnet.
    //
    // Multi-AZ expansion trigger: when SLA requirement exceeds 99.9%.
    // To expand: add AZ entries to config. CDK creates new subnets, route tables,
    // and NAT Gateways (one per AZ) automatically. No VPC replacement needed.
    const vpc = new ec2.Vpc(this, 'Vpc', {
      ipAddresses: ec2.IpAddresses.cidr(config.vpcCidr),
      availabilityZones: [...config.availabilityZones],
      natGateways: config.availabilityZones.length,
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

    // Lambda — API handler, Ledger Processor, Event Interpreter
    const lambdaSg = new ec2.SecurityGroup(this, 'LambdaSg', {
      vpc,
      description:
        'Lambda functions (API, Ledger Processor, Event Interpreter)',
      allowAllOutbound: false,
    });

    // RDS — PostgreSQL (accessed by Lambda and ECS)
    const rdsSg = new ec2.SecurityGroup(this, 'RdsSg', {
      vpc,
      description: 'RDS PostgreSQL instance',
      allowAllOutbound: false,
    });

    // ECS — Fargate tasks (Galexie live + backfill)
    const ecsSg = new ec2.SecurityGroup(this, 'EcsSg', {
      vpc,
      description: 'ECS Fargate tasks (Galexie ingestion)',
      allowAllOutbound: false,
    });

    // ---------------------
    // Lambda SG rules
    // ---------------------
    // Outbound to RDS on PostgreSQL port (will also cover RDS Proxy when added)
    lambdaSg.addEgressRule(
      rdsSg,
      ec2.Port.tcp(5432),
      'Allow Lambda → RDS on PostgreSQL port'
    );
    // Outbound HTTPS — covers AWS service API calls (Secrets Manager,
    // CloudWatch, X-Ray) and S3 via VPC endpoint (routed at route-table level)
    lambdaSg.addEgressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(443),
      'Allow Lambda → HTTPS (AWS APIs, S3 via VPC endpoint)'
    );

    // ---------------------
    // RDS SG rules
    // ---------------------
    // Inbound from Lambda
    rdsSg.addIngressRule(
      lambdaSg,
      ec2.Port.tcp(5432),
      'Allow Lambda → RDS on PostgreSQL port'
    );
    // Inbound from ECS (Galexie may need direct DB access for health checks)
    rdsSg.addIngressRule(
      ecsSg,
      ec2.Port.tcp(5432),
      'Allow ECS → RDS on PostgreSQL port'
    );

    // ---------------------
    // ECS SG rules
    // ---------------------
    // Outbound HTTPS — ECR image pull, CloudWatch Logs, S3 via VPC endpoint
    ecsSg.addEgressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(443),
      'Allow ECS → HTTPS (ECR, CloudWatch, S3 via VPC endpoint)'
    );
    // Outbound to RDS on PostgreSQL port
    ecsSg.addEgressRule(
      rdsSg,
      ec2.Port.tcp(5432),
      'Allow ECS → RDS on PostgreSQL port'
    );
    // Outbound for Stellar network peer connections and history archive access.
    // Galexie connects to Stellar peers on port 11625 (overlay protocol)
    // and history archives via HTTPS (covered by the 443 rule above).
    ecsSg.addEgressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(11625),
      'Allow ECS → Stellar peer network (overlay protocol)'
    );

    this.lambdaSecurityGroup = lambdaSg;
    this.rdsSecurityGroup = rdsSg;
    this.ecsSecurityGroup = ecsSg;

    // ---------------------
    // S3 Gateway VPC Endpoint
    // ---------------------
    // Gateway type (free, no hourly cost). Adds route table entries so
    // S3 traffic from private subnets stays within AWS network instead of
    // traversing the NAT Gateway — reduces cost and improves latency.
    vpc.addGatewayEndpoint('S3Endpoint', {
      service: ec2.GatewayVpcEndpointAwsService.S3,
      subnets: [{ subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS }],
    });

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
    new cdk.CfnOutput(this, 'RdsSecurityGroupId', {
      value: rdsSg.securityGroupId,
    });
    new cdk.CfnOutput(this, 'EcsSecurityGroupId', {
      value: ecsSg.securityGroupId,
    });
  }
}
