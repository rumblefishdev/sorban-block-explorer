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

  // Storage (consumed by StorageStack)
  readonly dbInstanceClass: string;
  readonly dbAllocatedStorage: number;
  readonly dbMultiAz: boolean;
  readonly dbDeletionProtection: boolean;
  readonly dbBackupRetentionDays: number;
  readonly kmsEncryption: boolean;
}
