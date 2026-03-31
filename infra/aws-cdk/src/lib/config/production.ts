import type { EnvironmentConfig } from './types.js';

export const productionConfig: EnvironmentConfig = {
  envName: 'production',
  awsRegion: 'us-east-1',

  // 10.1.0.0/16 — distinct from staging (10.0.0.0/16)
  vpcCidr: '10.1.0.0/16',
  availabilityZones: ['us-east-1a'],
  natType: 'gateway',
};
