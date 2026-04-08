import * as cdk from 'aws-cdk-lib';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as wafv2 from 'aws-cdk-lib/aws-wafv2';
import { Construct } from 'constructs';

export type WafScope = 'CLOUDFRONT' | 'REGIONAL';

export interface WafWebAclProps {
  /**
   * WAF scope. CLOUDFRONT WebACLs can only be attached to CloudFront
   * distributions; REGIONAL WebACLs to API Gateway, ALB, AppSync.
   * One WebACL cannot serve both scopes — that is an AWS WAF design
   * constraint, not a CDK limitation.
   */
  readonly scope: WafScope;

  /**
   * Stable, human-readable name. Becomes part of the WebACL name and
   * CloudWatch metric/log names. Must be unique within scope.
   */
  readonly name: string;

  /**
   * Per-IP request limit over a 5-minute window. AWS rate-based rules
   * cannot use a shorter window. For browser-facing distributions a
   * higher limit (5000+) is appropriate; for backend APIs lower (1000-2000).
   */
  readonly rateLimit: number;

  /**
   * CloudWatch Logs retention for WAF logs. Defaults to 1 month if omitted.
   */
  readonly logRetention?: logs.RetentionDays;
}

/**
 * Reusable WAF WebACL with the same managed rule set used across the
 * project: AWS Common Rule Set, Known Bad Inputs, IP Reputation List,
 * plus a per-IP rate limit. Includes CloudWatch Logs delivery with the
 * required resource policy on the log group (CDK does not auto-create
 * this for `CfnLoggingConfiguration`).
 *
 * Centralizing this here keeps the CloudFront-scoped and REGIONAL-scoped
 * ACLs identical in rule semantics — when one needs tuning, both get the
 * same change.
 */
export class WafWebAcl extends Construct {
  readonly webAcl: wafv2.CfnWebACL;
  readonly logGroup: logs.LogGroup;

  constructor(scope: Construct, id: string, props: WafWebAclProps) {
    super(scope, id);

    const { account, region } = cdk.Stack.of(this);

    // CloudWatch Logs delivery for WAF requires the log group name to
    // start with `aws-waf-logs-`.
    this.logGroup = new logs.LogGroup(this, 'LogGroup', {
      logGroupName: `aws-waf-logs-${props.name}`,
      retention: props.logRetention ?? logs.RetentionDays.ONE_MONTH,
      removalPolicy: cdk.RemovalPolicy.DESTROY,
    });

    this.webAcl = new wafv2.CfnWebACL(this, 'WebAcl', {
      name: props.name,
      scope: props.scope,
      defaultAction: { allow: {} },
      visibilityConfig: {
        cloudWatchMetricsEnabled: true,
        metricName: props.name,
        sampledRequestsEnabled: true,
      },
      rules: [
        {
          name: 'AWSManagedRulesCommonRuleSet',
          priority: 1,
          overrideAction: { none: {} },
          statement: {
            managedRuleGroupStatement: {
              vendorName: 'AWS',
              name: 'AWSManagedRulesCommonRuleSet',
            },
          },
          visibilityConfig: {
            cloudWatchMetricsEnabled: true,
            metricName: 'CommonRuleSet',
            sampledRequestsEnabled: true,
          },
        },
        {
          name: 'AWSManagedRulesKnownBadInputsRuleSet',
          priority: 2,
          overrideAction: { none: {} },
          statement: {
            managedRuleGroupStatement: {
              vendorName: 'AWS',
              name: 'AWSManagedRulesKnownBadInputsRuleSet',
            },
          },
          visibilityConfig: {
            cloudWatchMetricsEnabled: true,
            metricName: 'KnownBadInputs',
            sampledRequestsEnabled: true,
          },
        },
        {
          name: 'AWSManagedRulesAmazonIpReputationList',
          priority: 3,
          overrideAction: { none: {} },
          statement: {
            managedRuleGroupStatement: {
              vendorName: 'AWS',
              name: 'AWSManagedRulesAmazonIpReputationList',
            },
          },
          visibilityConfig: {
            cloudWatchMetricsEnabled: true,
            metricName: 'IpReputation',
            sampledRequestsEnabled: true,
          },
        },
        {
          name: 'RateLimit',
          priority: 4,
          action: { block: {} },
          statement: {
            rateBasedStatement: {
              limit: props.rateLimit,
              aggregateKeyType: 'IP',
            },
          },
          visibilityConfig: {
            cloudWatchMetricsEnabled: true,
            metricName: 'RateLimit',
            sampledRequestsEnabled: true,
          },
        },
      ],
    });

    // Resource policy granting WAF log delivery service permission to
    // PutLogEvents on this log group. CDK does NOT auto-create this for
    // `CfnLoggingConfiguration`. SourceAccount + SourceArn conditions
    // mitigate confused deputy across accounts (defense in depth, even
    // though we are single-account today).
    new logs.CfnResourcePolicy(this, 'LogResourcePolicy', {
      policyName: `${props.name}-log-delivery`,
      policyDocument: JSON.stringify({
        Version: '2012-10-17',
        Statement: [
          {
            Effect: 'Allow',
            Principal: { Service: 'delivery.logs.amazonaws.com' },
            Action: ['logs:CreateLogStream', 'logs:PutLogEvents'],
            Resource: `${this.logGroup.logGroupArn}:*`,
            Condition: {
              StringEquals: { 'aws:SourceAccount': account },
              ArnLike: {
                'aws:SourceArn': `arn:aws:logs:${region}:${account}:*`,
              },
            },
          },
        ],
      }),
    });

    new wafv2.CfnLoggingConfiguration(this, 'Logging', {
      logDestinationConfigs: [this.logGroup.logGroupArn],
      resourceArn: this.webAcl.attrArn,
    });
  }

  /** ARN of the WebACL — convenience accessor. */
  get webAclArn(): string {
    return this.webAcl.attrArn;
  }
}
