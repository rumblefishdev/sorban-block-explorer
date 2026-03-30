import { resolveEnvironment } from './config.js';

/**
 * Resolves the PostgreSQL connection string from the environment.
 *
 * - Dev: reads DATABASE_URL env var directly.
 * - Staging/Production: fetches from AWS Secrets Manager using DATABASE_SECRET_ARN.
 *   DATABASE_URL is ignored in non-dev environments to prevent accidental
 *   use of local credentials in staging/production.
 *   The SDK is dynamically imported so it is never loaded in dev.
 */
export async function resolveConnectionString(): Promise<string> {
  const env = resolveEnvironment();

  if (env === 'dev') {
    const databaseUrl = process.env['DATABASE_URL'];
    if (databaseUrl) return databaseUrl;
    throw new Error('DATABASE_URL must be set in dev environment.');
  }

  const secretArn = process.env['DATABASE_SECRET_ARN'];
  const region = process.env['AWS_REGION'] ?? 'us-east-1';

  if (!secretArn) {
    throw new Error(
      `DATABASE_SECRET_ARN must be set in ${env} environment. ` +
        'DATABASE_URL is only allowed in dev.'
    );
  }

  let SecretsManagerClient: typeof import('@aws-sdk/client-secrets-manager').SecretsManagerClient;
  let GetSecretValueCommand: typeof import('@aws-sdk/client-secrets-manager').GetSecretValueCommand;

  try {
    const mod = await import('@aws-sdk/client-secrets-manager');
    SecretsManagerClient = mod.SecretsManagerClient;
    GetSecretValueCommand = mod.GetSecretValueCommand;
  } catch {
    throw new Error(
      'Failed to load @aws-sdk/client-secrets-manager. ' +
        'This dependency must be available when DATABASE_SECRET_ARN is set. ' +
        'Ensure it is installed or bundled in non-dev environments.'
    );
  }

  const client = new SecretsManagerClient({ region });
  const response = await client.send(
    new GetSecretValueCommand({ SecretId: secretArn })
  );

  if (!response.SecretString) {
    throw new Error(`Secret ${secretArn} has no string value`);
  }

  let secret: Record<string, unknown>;
  try {
    secret = JSON.parse(response.SecretString) as Record<string, unknown>;
  } catch (err) {
    throw new Error(
      `Secret ${secretArn} must contain valid JSON: ${(err as Error).message}`
    );
  }

  const host = secret['host'];
  const port = secret['port'];
  const username = secret['username'];
  const password = secret['password'];
  const dbname = secret['dbname'];

  if (
    typeof host !== 'string' ||
    typeof port !== 'number' ||
    typeof username !== 'string' ||
    typeof password !== 'string' ||
    typeof dbname !== 'string'
  ) {
    throw new Error(
      `Secret ${secretArn} does not match expected RDS format (host, port, username, password, dbname)`
    );
  }

  return `postgres://${encodeURIComponent(username)}:${encodeURIComponent(
    password
  )}@${host}:${port}/${encodeURIComponent(dbname)}`;
}
