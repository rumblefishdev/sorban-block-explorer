import { Module, Global } from '@nestjs/common';
import { ConfigService } from '@nestjs/config';
import { Pool } from 'pg';
import { drizzle } from 'drizzle-orm/node-postgres';

export const DATABASE_CONNECTION = Symbol('DATABASE_CONNECTION');

@Global()
@Module({
  providers: [
    {
      provide: DATABASE_CONNECTION,
      useFactory: async (config: ConfigService) => {
        const pool = new Pool({
          host: config.get<string>('RDS_PROXY_HOST', 'localhost'),
          database: config.get<string>('DB_NAME', 'soroban_explorer'),
          user: config.get<string>('DB_USER', 'postgres'),
          password: config.get<string>('DB_PASSWORD', 'postgres'),
          port: Number(config.get('DB_PORT', '5432')),
          max: 1,
          min: 0,
          idleTimeoutMillis: 120_000,
          connectionTimeoutMillis: 10_000,
          ssl:
            config.get<string>('NODE_ENV') === 'production'
              ? { rejectUnauthorized: true }
              : undefined,
        });

        return drizzle({ client: pool });
      },
      inject: [ConfigService],
    },
  ],
  exports: [DATABASE_CONNECTION],
})
export class DatabaseModule {}
