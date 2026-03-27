import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'drizzle-kit';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  dialect: 'postgresql',
  schema: path.resolve(__dirname, 'src/schema/index.ts'),
  out: path.resolve(__dirname, 'drizzle'),
  dbCredentials: {
    url:
      process.env['DATABASE_URL'] ??
      'postgres://postgres:postgres@localhost:5432/soroban_block_explorer',
  },
  verbose: true,
  strict: true,
});
