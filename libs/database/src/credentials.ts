/**
 * Resolves the PostgreSQL connection string from the environment.
 *
 * - Dev: reads DATABASE_URL env var directly.
 * - Staging/Production: fetches from AWS Secrets Manager using DATABASE_SECRET_ARN.
 *   The SDK is dynamically imported so it is never loaded in dev.
 */
export async function resolveConnectionString(): Promise<string> {
  const databaseUrl = process.env['DATABASE_URL'];
  if (databaseUrl) return databaseUrl;

  const secretArn = process.env['DATABASE_SECRET_ARN'];
  const region = process.env['AWS_REGION'] ?? 'us-east-1';

  if (!secretArn) {
    throw new Error(
      'DATABASE_URL or DATABASE_SECRET_ARN must be set. ' +
        'Set DATABASE_URL for local dev, or DATABASE_SECRET_ARN for staging/production.'
    );
  }

  const { SecretsManagerClient, GetSecretValueCommand } = await import(
    '@aws-sdk/client-secrets-manager'
  );

  const client = new SecretsManagerClient({ region });
  const response = await client.send(
    new GetSecretValueCommand({ SecretId: secretArn })
  );

  if (!response.SecretString) {
    throw new Error(`Secret ${secretArn} has no string value`);
  }

  const secret: Record<string, unknown> = JSON.parse(response.SecretString);

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
  )}@${host}:${port}/${dbname}`;
}
