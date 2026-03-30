import { drizzle } from 'drizzle-orm/node-postgres';
import type { NodePgDatabase } from 'drizzle-orm/node-postgres';
import * as pg from 'pg';

import { resolveEnvironment, type DatabaseConfig } from './config.js';
import { resolveConnectionString } from './credentials.js';
import * as schema from './schema/index.js';

const { Pool } = pg;

// Module-level cache for Lambda warm invocation reuse
let _pool: pg.Pool | undefined;
let _db: NodePgDatabase<typeof schema> | undefined;
let _initPromise: Promise<NodePgDatabase<typeof schema>> | undefined;

export type Database = NodePgDatabase<typeof schema>;

async function buildConfig(): Promise<DatabaseConfig> {
  const env = resolveEnvironment();
  const connectionString = await resolveConnectionString();

  const ssl: DatabaseConfig['ssl'] =
    env === 'production'
      ? { rejectUnauthorized: true }
      : env === 'staging'
      ? { rejectUnauthorized: false }
      : false;

  return {
    connectionString,
    ssl,
    pool: {
      max: 1,
      min: 0,
      idleTimeoutMillis: 120_000,
      connectionTimeoutMillis: 10_000,
    },
  };
}

async function initDb(): Promise<NodePgDatabase<typeof schema>> {
  const config = await buildConfig();

  _pool = new Pool({
    connectionString: config.connectionString,
    ssl: config.ssl,
    ...config.pool,
  });

  _db = drizzle({ client: _pool, schema });
  return _db;
}

/**
 * Returns the singleton Drizzle database client.
 *
 * On first call: resolves credentials, creates a pg.Pool, wraps with drizzle().
 * On subsequent calls: returns the cached instance (Lambda warm reuse).
 * Concurrent calls during init share the same promise (no race condition).
 */
export async function getDb(): Promise<Database> {
  if (_db) return _db;

  if (!_initPromise) {
    _initPromise = initDb().catch((err) => {
      // Reset so next invocation retries instead of returning cached rejection
      _initPromise = undefined;
      throw err;
    });
  }

  return _initPromise;
}

/**
 * Returns the underlying pg.Pool. Useful for health checks or raw queries.
 * Throws if getDb() has not been called yet.
 */
export function getPool(): pg.Pool {
  if (!_pool) {
    throw new Error('Database not initialized. Call getDb() first.');
  }
  return _pool;
}

/**
 * Closes the database connection pool. For graceful shutdown in tests or
 * non-Lambda environments.
 */
export async function closeDb(): Promise<void> {
  if (_pool) {
    await _pool.end();
    _pool = undefined;
    _db = undefined;
    _initPromise = undefined;
  }
}
