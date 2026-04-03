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
}
