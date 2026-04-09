/**
 * Environment-specific configuration for the CDK infrastructure.
 *
 * Only includes fields consumed by existing stacks. Each new stack task
 * extends this interface with the fields it needs — no placeholder values.
 */
export interface EnvironmentConfig {
  readonly envName: 'staging' | 'production';
  readonly awsRegion: string;

  // Network (consumed by NetworkStack)
  readonly vpcCidr: string;
  readonly availabilityZones: readonly string[];
  readonly natType: 'gateway' | 'instance';
  readonly natGatewayCount: number;

  // Storage (consumed by RdsStack, LedgerBucketStack)
  readonly dbInstanceClass: string;
  readonly dbAllocatedStorage: number;
  readonly dbMultiAz: boolean;
  readonly dbDeletionProtection: boolean;
  readonly dbBackupRetentionDays: number;
  readonly dbProxy: boolean;
  readonly kmsEncryption: boolean;

  // Compute (consumed by ComputeStack)
  readonly apiLambdaMemory: number;
  readonly apiLambdaTimeout: number;
  readonly indexerLambdaMemory: number;
  readonly indexerLambdaTimeout: number;
  readonly indexerLambdaConcurrency: number;

  // Ingestion — ECS Fargate (consumed by IngestionStack)

  /** Fargate CPU units for Galexie tasks (256, 512, 1024, 2048, 4096). */
  readonly galexieCpu: number;
  /** Fargate memory in MiB for Galexie tasks. Must be compatible with CPU — see https://docs.aws.amazon.com/AmazonECS/latest/developerguide/task-cpu-memory-error.html */
  readonly galexieMemory: number;
  /** Ephemeral storage in GiB (21–200). Captive Core needs local disk for ledger catchup. */
  readonly galexieEphemeralStorage: number;
  /** Desired count for the Galexie live service (typically 1 — single writer). */
  readonly galexieDesiredCount: number;
  /** Stellar network passphrase. Determines which network Galexie connects to. */
  readonly stellarNetworkPassphrase: string;
  /** CloudWatch Logs retention in days for ECS log groups. */
  readonly ecsLogRetentionDays: number;
  /** Graceful shutdown timeout in seconds. ECS waits this long after SIGTERM before SIGKILL. */
  readonly galexieStopTimeout: number;
  /** Enable ECS Exec (shell access into containers via SSM). Adds ssmmessages IAM permissions. */
  readonly ecsExecEnabled: boolean;
  /**
   * ECR image tag for Galexie container. Defaults to "latest" until CI/CD
   * pipeline (task 0039) is implemented — once available, each deploy will
   * set this to a git SHA for immutable, reproducible deployments.
   */
  readonly galexieImageTag: string;
  /** Whether to create the backfill task definition. Not every environment needs backfill. */
  readonly galexieBackfillEnabled: boolean;

  // API Gateway (consumed by ApiGatewayStack)

  /** Sustained requests per second before API Gateway returns 429. */
  readonly apiGatewayThrottleRate: number;
  /** Maximum concurrent requests allowed in a short burst above the rate limit. */
  readonly apiGatewayThrottleBurst: number;
  /** Whether to provision a dedicated cache cluster (Memcached) on the stage. false = no cluster, no cost. */
  readonly apiGatewayCacheEnabled: boolean;
  /** Cache cluster size in GB. String because AWS API accepts '0.5', '1.6', '6.1', etc. */
  readonly apiGatewayCacheSize: string;
  /** Cache TTL (seconds) for immutable endpoints (e.g. /transactions/{hash}). Not yet wired per-method — awaits route patterns from task 0033. */
  readonly apiGatewayCacheTtlImmutable: number;
  /** Cache TTL (seconds) for mutable endpoints (e.g. /transactions?limit=20). Used as the stage-level default. */
  readonly apiGatewayCacheTtlMutable: number;
  /** Daily request quota for partner API key usage plans. */
  readonly apiGatewayPartnerDailyQuota: number;

  // Delivery (consumed by DeliveryStack + ApiGatewayStack)

  /** Frontend SPA domain, e.g. "staging.sorobanscan.rumblefish.dev". */
  readonly domainName: string;
  /** API custom domain, e.g. "api.staging.sorobanscan.rumblefish.dev". */
  readonly apiDomainName: string;
  /** Existing Route 53 hosted zone ID for sorobanscan.rumblefish.dev. */
  readonly hostedZoneId: string;
  /** Hosted zone name, e.g. "sorobanscan.rumblefish.dev". */
  readonly hostedZoneName: string;
  /** ACM wildcard certificate ARN in us-east-1 covering *.sorobanscan.rumblefish.dev. */
  readonly certificateArn: string;
  /**
   * Provision WAF WebACLs (one CLOUDFRONT-scoped on the distribution,
   * one REGIONAL-scoped on the API Gateway stage). Disable on staging
   * to save the ~$15-20/month fixed cost when basic auth gating is
   * the primary access control.
   */
  readonly enableWaf: boolean;
  /**
   * Enable CloudFront Function basic auth on the SPA distribution.
   * Credentials live in CloudFront KeyValueStore — see DeliveryStack
   * for the bootstrap procedure. Production should leave this false.
   */
  readonly enableBasicAuth: boolean;
  /**
   * Per-IP request limit over a 5-minute window for the CloudFront WAF.
   * Browser-facing — needs to be high enough to accommodate normal SPA
   * page loads (50-100 asset requests). Suggested: 5000+ for production,
   * lower for staging.
   */
  readonly cloudFrontWafRateLimit: number;
  /**
   * Per-IP request limit over a 5-minute window for the API Gateway WAF.
   * Should reflect realistic API usage; lower than the CloudFront limit.
   * Suggested: 1000-2000.
   */
  readonly apiWafRateLimit: number;

  // Observability — X-Ray (consumed by ObservabilityStack)

  /** X-Ray sampling rate (0.0–1.0). Lower in production to reduce cost. */
  readonly xraySamplingRate: number;
  /** X-Ray reservoir size — fixed traces/sec guaranteed before sampling kicks in. */
  readonly xrayReservoirSize: number;
}

/**
 * Returns the record name relative to the hosted zone, suitable for
 * `recordName` on `route53.ARecord` / `AaaaRecord`.
 *
 * CDK Route 53 record constructs concatenate `recordName` with the
 * hosted zone name unless `recordName` ends with a trailing dot. Passing
 * a full FQDN like `staging.sorobanscan.rumblefish.dev` (no trailing dot)
 * against a zone `sorobanscan.rumblefish.dev` therefore produces a
 * broken record `staging.sorobanscan.rumblefish.dev.sorobanscan.rumblefish.dev`.
 *
 * This helper strips the zone suffix so callers always get a relative
 * label. For an apex record (`fqdn === zoneName`) it returns the zone
 * name itself, which CDK accepts as the apex.
 */
export function relativeRecordName(fqdn: string, zoneName: string): string {
  if (fqdn === zoneName) {
    return zoneName;
  }
  const suffix = `.${zoneName}`;
  if (!fqdn.endsWith(suffix)) {
    throw new Error(
      `relativeRecordName: "${fqdn}" is not within zone "${zoneName}"`
    );
  }
  return fqdn.slice(0, -suffix.length);
}

/**
 * Validates an EnvironmentConfig at synth time. Throws on missing or
 * placeholder values rather than letting `cdk synth`/`cdk deploy` fail
 * deep inside CloudFormation with cryptic errors.
 *
 * Catches the most common footgun: a placeholder like `CHANGE_ME`
 * accidentally committed to envs/*.json.
 */
export function validateConfig(config: EnvironmentConfig): void {
  const errors: string[] = [];

  if (
    !/^arn:aws:acm:[a-z0-9-]+:\d{12}:certificate\//.test(config.certificateArn)
  ) {
    errors.push(
      `certificateArn must be a valid ACM certificate ARN, got: "${config.certificateArn}"`
    );
  }
  if (!/^Z[A-Z0-9]+$/.test(config.hostedZoneId)) {
    errors.push(
      `hostedZoneId must be a Route 53 hosted zone ID (e.g. "Z1234ABCD"), got: "${config.hostedZoneId}"`
    );
  }
  if (!config.hostedZoneName || config.hostedZoneName.includes('CHANGE')) {
    errors.push(
      `hostedZoneName missing or placeholder: "${config.hostedZoneName}"`
    );
  }
  if (!config.domainName || config.domainName.includes('CHANGE')) {
    errors.push(`domainName missing or placeholder: "${config.domainName}"`);
  }
  if (!config.apiDomainName || config.apiDomainName.includes('CHANGE')) {
    errors.push(
      `apiDomainName missing or placeholder: "${config.apiDomainName}"`
    );
  }
  if (config.cloudFrontWafRateLimit < 100) {
    errors.push(
      `cloudFrontWafRateLimit must be >= 100 (AWS WAF minimum), got: ${config.cloudFrontWafRateLimit}`
    );
  }
  if (config.apiWafRateLimit < 100) {
    errors.push(
      `apiWafRateLimit must be >= 100 (AWS WAF minimum), got: ${config.apiWafRateLimit}`
    );
  }
  if (config.xraySamplingRate < 0 || config.xraySamplingRate > 1) {
    errors.push(
      `xraySamplingRate must be between 0.0 and 1.0, got: ${config.xraySamplingRate}`
    );
  }
  if (
    !Number.isInteger(config.xrayReservoirSize) ||
    config.xrayReservoirSize < 0
  ) {
    errors.push(
      `xrayReservoirSize must be a non-negative integer, got: ${config.xrayReservoirSize}`
    );
  }

  if (errors.length > 0) {
    throw new Error(
      `Invalid EnvironmentConfig for "${config.envName}":\n  - ${errors.join(
        '\n  - '
      )}`
    );
  }

  // Soft sanity check: an environment with neither WAF nor basic auth
  // exposes an unprotected public CloudFront distribution. That may be
  // intentional for production (relying on application-layer controls),
  // but is almost always a mistake on staging — flag it loudly.
  if (!config.enableWaf && !config.enableBasicAuth) {
    // eslint-disable-next-line no-console
    console.warn(
      `[validateConfig] WARNING: ${config.envName} has both enableWaf=false and enableBasicAuth=false. ` +
        `The CloudFront distribution will be publicly accessible with no gating. ` +
        `If this is intentional, ignore. Otherwise enable one of them in envs/${config.envName}.json.`
    );
  }
}

/** Configuration for the shared CI/CD stack (consumed by CicdStack). */
export interface CicdConfig {
  readonly awsRegion: string;
  /** GitHub org/repo, e.g. "rumblefishdev/soroban-block-explorer" */
  readonly githubRepo: string;
}
