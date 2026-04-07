import * as cdk from 'aws-cdk-lib';
import * as acm from 'aws-cdk-lib/aws-certificatemanager';
import * as cloudfront from 'aws-cdk-lib/aws-cloudfront';
import * as origins from 'aws-cdk-lib/aws-cloudfront-origins';
import * as route53 from 'aws-cdk-lib/aws-route53';
import * as targets from 'aws-cdk-lib/aws-route53-targets';
import * as s3 from 'aws-cdk-lib/aws-s3';
import type { Construct } from 'constructs';

import { basicAuthFunctionCode } from '../cloudfront-functions/basic-auth.js';
import { WafWebAcl } from '../constructs/waf-web-acl.js';
import { relativeRecordName, type EnvironmentConfig } from '../types.js';

export interface DeliveryStackProps extends cdk.StackProps {
  readonly config: EnvironmentConfig;
}

/**
 * Public delivery stack for the Soroban Block Explorer (frontend).
 *
 * Creates:
 * - S3 bucket for React SPA static hosting (private, CloudFront OAC)
 * - CloudFront distribution with SPA routing fallback
 * - WAF WebACL (CLOUDFRONT scope) attached to the distribution — gated
 *   by `config.enableWaf`
 * - Route 53 DNS records for frontend
 * - Optional CloudFront Function basic auth gating — see `config.enableBasicAuth`
 *
 * The API Gateway has its own REGIONAL WebACL defined in `ApiGatewayStack`.
 * A single WebACL cannot serve both CLOUDFRONT and REGIONAL scopes —
 * an AWS WAF design constraint. Both stacks instantiate `WafWebAcl` from
 * `lib/constructs/waf-web-acl.ts` to keep rule sets in lockstep.
 */
export class DeliveryStack extends cdk.Stack {
  readonly distribution: cloudfront.Distribution;

  constructor(scope: Construct, id: string, props: DeliveryStackProps) {
    super(scope, id, props);

    const { config } = props;

    // ---------------------
    // WAF (CLOUDFRONT scope) — optional
    // ---------------------
    const waf = config.enableWaf
      ? new WafWebAcl(this, 'Waf', {
          scope: 'CLOUDFRONT',
          name: `${config.envName}-soroban-explorer-cf`,
          rateLimit: config.cloudFrontWafRateLimit,
        })
      : undefined;

    // ---------------------
    // S3 Bucket (SPA)
    // ---------------------
    const spaBucket = new s3.Bucket(this, 'SpaBucket', {
      bucketName: `${config.envName}-soroban-explorer-spa`,
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      encryption: s3.BucketEncryption.S3_MANAGED,
      removalPolicy:
        config.envName === 'production'
          ? cdk.RemovalPolicy.RETAIN
          : cdk.RemovalPolicy.DESTROY,
      autoDeleteObjects: config.envName !== 'production',
    });

    // ---------------------
    // ACM Certificate (import existing)
    // ---------------------
    const certificate = acm.Certificate.fromCertificateArn(
      this,
      'Certificate',
      config.certificateArn
    );

    // ---------------------
    // CloudFront Function (staging basic auth) — backed by KeyValueStore
    // ---------------------
    // Credentials live in a CloudFront KeyValueStore, NOT in code, NOT in
    // git, NOT in CFN template. Populate the KVS once after deploy:
    //
    //   KVS_ARN=$(aws cloudfront list-key-value-stores --query "KeyValueStoreList.Items[?Name=='staging-soroban-explorer-basic-auth'].ARN" --output text)
    //   ETAG=$(aws cloudfront-keyvaluestore describe-key-value-store --kvs-arn "$KVS_ARN" --query "ETag" --output text)
    //   TOKEN=$(printf 'staging:<password>' | base64)
    //   aws cloudfront-keyvaluestore put-key --kvs-arn "$KVS_ARN" \
    //     --key auth-token --value "$TOKEN" --if-match "$ETAG"
    //
    // Rotation: rerun put-key with a new password — change is live within
    // ~seconds, no CDK deploy needed.
    //
    // First-deploy gotcha: KVS is empty until you populate it. Until then,
    // requests fail closed and the function returns 503 (safer than open).
    let viewerRequestFunction: cloudfront.Function | undefined;

    if (config.enableBasicAuth) {
      const basicAuthKvs = new cloudfront.KeyValueStore(this, 'BasicAuthKvs', {
        keyValueStoreName: `${config.envName}-soroban-explorer-basic-auth`,
      });

      viewerRequestFunction = new cloudfront.Function(
        this,
        'BasicAuthFunction',
        {
          functionName: `${config.envName}-soroban-explorer-basic-auth`,
          keyValueStore: basicAuthKvs,
          runtime: cloudfront.FunctionRuntime.JS_2_0,
          code: cloudfront.FunctionCode.fromInline(
            basicAuthFunctionCode(basicAuthKvs.keyValueStoreId)
          ),
        }
      );

      new cdk.CfnOutput(this, 'BasicAuthKvsArn', {
        value: basicAuthKvs.keyValueStoreArn,
      });
    }

    // ---------------------
    // Response Headers Policy — security baseline
    // ---------------------
    // Custom policy rather than the AWS-managed `SECURITY_HEADERS`,
    // because the managed policy includes a strict CSP (`default-src
    // 'self'`) that breaks typical SPAs loading fonts/assets from CDNs.
    // CSP is intentionally omitted here — it should be added in a
    // follow-up task once the frontend is deployed and we know what it
    // actually loads (then dial in via Content-Security-Policy-Report-Only
    // before enforcing).
    const responseHeadersPolicy = new cloudfront.ResponseHeadersPolicy(
      this,
      'ResponseHeadersPolicy',
      {
        responseHeadersPolicyName: `${config.envName}-soroban-explorer-headers`,
        securityHeadersBehavior: {
          strictTransportSecurity: {
            // 1 year on production, 1 week on staging. Short max-age on
            // staging means HSTS misconfiguration is recoverable within
            // days rather than a year.
            accessControlMaxAge:
              config.envName === 'production'
                ? cdk.Duration.days(365)
                : cdk.Duration.days(7),
            // Only on production: staging is itself a subdomain, so
            // includeSubdomains here would force ALL sibling subdomains
            // (sandbox, dev, internal tools) to HTTPS and break legit
            // dev workflows.
            includeSubdomains: config.envName === 'production',
            // Never set automatically: HSTS preload submission to
            // hstspreload.org is a one-way door — removal takes months
            // and is not guaranteed. Enable explicitly only after a
            // production launch decision and security sign-off.
            preload: false,
            override: true,
          },
          contentTypeOptions: { override: true },
          frameOptions: {
            frameOption: cloudfront.HeadersFrameOption.DENY,
            override: true,
          },
          referrerPolicy: {
            referrerPolicy:
              cloudfront.HeadersReferrerPolicy.STRICT_ORIGIN_WHEN_CROSS_ORIGIN,
            override: true,
          },
        },
      }
    );

    // ---------------------
    // CloudFront Distribution
    // ---------------------
    const distribution = new cloudfront.Distribution(this, 'Distribution', {
      domainNames: [config.domainName],
      certificate,
      defaultRootObject: 'index.html',
      priceClass: cloudfront.PriceClass.PRICE_CLASS_100,
      ...(waf && { webAclId: waf.webAclArn }),
      minimumProtocolVersion: cloudfront.SecurityPolicyProtocol.TLS_V1_2_2021,
      httpVersion: cloudfront.HttpVersion.HTTP2_AND_3,
      defaultBehavior: {
        origin: origins.S3BucketOrigin.withOriginAccessControl(spaBucket),
        viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS,
        cachePolicy: cloudfront.CachePolicy.CACHING_OPTIMIZED,
        responseHeadersPolicy,
        ...(viewerRequestFunction && {
          functionAssociations: [
            {
              function: viewerRequestFunction,
              eventType: cloudfront.FunctionEventType.VIEWER_REQUEST,
            },
          ],
        }),
      },
      errorResponses: [
        {
          httpStatus: 403,
          responseHttpStatus: 200,
          responsePagePath: '/index.html',
          ttl: cdk.Duration.seconds(0),
        },
        {
          httpStatus: 404,
          responseHttpStatus: 200,
          responsePagePath: '/index.html',
          ttl: cdk.Duration.seconds(0),
        },
      ],
    });

    this.distribution = distribution;

    // ---------------------
    // Route 53 (frontend)
    // ---------------------
    const hostedZone = route53.HostedZone.fromHostedZoneAttributes(
      this,
      'HostedZone',
      {
        hostedZoneId: config.hostedZoneId,
        zoneName: config.hostedZoneName,
      }
    );

    // recordName must be RELATIVE to the hosted zone — CDK concatenates
    // it with zoneName unless it ends in a trailing dot. See
    // relativeRecordName() in types.ts.
    const frontendRecordName = relativeRecordName(
      config.domainName,
      config.hostedZoneName
    );

    new route53.ARecord(this, 'FrontendARecord', {
      zone: hostedZone,
      recordName: frontendRecordName,
      target: route53.RecordTarget.fromAlias(
        new targets.CloudFrontTarget(distribution)
      ),
    });

    new route53.AaaaRecord(this, 'FrontendAaaaRecord', {
      zone: hostedZone,
      recordName: frontendRecordName,
      target: route53.RecordTarget.fromAlias(
        new targets.CloudFrontTarget(distribution)
      ),
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
    new cdk.CfnOutput(this, 'DistributionDomainName', {
      value: distribution.distributionDomainName,
    });
    new cdk.CfnOutput(this, 'SpaBucketName', {
      value: spaBucket.bucketName,
    });
    if (waf) {
      new cdk.CfnOutput(this, 'CloudFrontWafWebAclArn', {
        value: waf.webAclArn,
      });
    }
  }
}
