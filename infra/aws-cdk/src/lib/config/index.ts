import { productionConfig } from './production.js';
import { stagingConfig } from './staging.js';
import type { EnvironmentConfig } from './types.js';

export type { EnvironmentConfig } from './types.js';

const configs: Record<string, EnvironmentConfig> = {
  staging: stagingConfig,
  production: productionConfig,
};

export function resolveConfig(envName?: string): EnvironmentConfig {
  const name = envName ?? process.env['CDK_ENV'] ?? 'staging';
  const config = configs[name];
  if (!config) {
    throw new Error(
      `Unknown environment "${name}". Valid: ${Object.keys(configs).join(', ')}`
    );
  }
  return config;
}
