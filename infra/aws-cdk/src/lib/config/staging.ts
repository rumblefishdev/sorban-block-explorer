import type { EnvironmentConfig } from './types.js';

export const stagingConfig: EnvironmentConfig = {
  envName: 'staging',
  awsRegion: 'us-east-1',

  // 10.0.0.0/16 — 65k IPs, room for Multi-AZ expansion
  vpcCidr: '10.0.0.0/16',
  availabilityZones: ['us-east-1a'],
  natType: 'instance',
};
