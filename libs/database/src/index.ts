export { getDb, getPool, closeDb, type Database } from './connection.js';
export {
  resolveEnvironment,
  type DatabaseConfig,
  type DatabaseEnvironment,
} from './config.js';
export * as schema from './schema/index.js';
