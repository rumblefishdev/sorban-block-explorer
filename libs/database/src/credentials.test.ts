import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { resolveConnectionString } from './credentials.js';

describe('resolveConnectionString', () => {
  const originalEnv = { ...process.env };

  beforeEach(() => {
    vi.unstubAllEnvs();
  });

  afterEach(() => {
    process.env = { ...originalEnv };
  });

  describe('dev environment', () => {
    beforeEach(() => {
      process.env['NODE_ENV'] = 'dev';
    });

    it('returns DATABASE_URL when set', async () => {
      process.env['DATABASE_URL'] = 'postgres://localhost/test';
      const result = await resolveConnectionString();
      expect(result).toBe('postgres://localhost/test');
    });

    it('throws when DATABASE_URL is not set', async () => {
      delete process.env['DATABASE_URL'];
      await expect(resolveConnectionString()).rejects.toThrow(
        'DATABASE_URL must be set in dev environment'
      );
    });
  });

  describe('staging environment', () => {
    beforeEach(() => {
      process.env['NODE_ENV'] = 'staging';
      delete process.env['DATABASE_URL'];
    });

    it('throws when DATABASE_SECRET_ARN is not set', async () => {
      delete process.env['DATABASE_SECRET_ARN'];
      await expect(resolveConnectionString()).rejects.toThrow(
        'DATABASE_SECRET_ARN must be set in staging environment'
      );
    });

    it('throws clear error when AWS SDK is not available', async () => {
      process.env['DATABASE_SECRET_ARN'] =
        'arn:aws:secretsmanager:us-east-1:123:secret:test';
      vi.doMock('@aws-sdk/client-secrets-manager', () => {
        throw new Error('Cannot find module');
      });
      await vi.resetModules();
      try {
        const { resolveConnectionString: fn } = await import(
          './credentials.js'
        );
        await expect(fn()).rejects.toThrow(
          /Failed to load @aws-sdk\/client-secrets-manager/
        );
      } finally {
        vi.doUnmock('@aws-sdk/client-secrets-manager');
        await vi.resetModules();
      }
    });
  });

  describe('production environment', () => {
    beforeEach(() => {
      process.env['NODE_ENV'] = 'production';
      delete process.env['DATABASE_URL'];
    });

    it('throws when DATABASE_SECRET_ARN is not set', async () => {
      delete process.env['DATABASE_SECRET_ARN'];
      await expect(resolveConnectionString()).rejects.toThrow(
        'DATABASE_SECRET_ARN must be set in production environment'
      );
    });

    it('ignores DATABASE_URL even if set', async () => {
      process.env['DATABASE_URL'] = 'postgres://localhost/should-be-ignored';
      delete process.env['DATABASE_SECRET_ARN'];
      await expect(resolveConnectionString()).rejects.toThrow(
        'DATABASE_SECRET_ARN must be set in production environment'
      );
    });
  });
});
