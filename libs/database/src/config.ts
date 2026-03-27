export type DatabaseEnvironment = 'dev' | 'staging' | 'production';

export interface DatabaseConfig {
  readonly connectionString: string;
  readonly ssl: boolean | { rejectUnauthorized: boolean };
  readonly pool: {
    readonly max: number;
    readonly min: number;
    readonly idleTimeoutMillis: number;
    readonly connectionTimeoutMillis: number;
  };
}

export function resolveEnvironment(): DatabaseEnvironment {
  const env = process.env['NODE_ENV'];
  if (env === 'production') return 'production';
  if (env === 'staging') return 'staging';
  return 'dev';
}
