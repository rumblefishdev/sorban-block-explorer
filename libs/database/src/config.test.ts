import { describe, it, expect, afterEach } from 'vitest';
import { resolveEnvironment } from './config.js';

describe('resolveEnvironment', () => {
  const originalEnv = process.env['NODE_ENV'];

  afterEach(() => {
    if (originalEnv === undefined) {
      delete process.env['NODE_ENV'];
    } else {
      process.env['NODE_ENV'] = originalEnv;
    }
  });

  it('returns "production" when NODE_ENV is "production"', () => {
    process.env['NODE_ENV'] = 'production';
    expect(resolveEnvironment()).toBe('production');
  });

  it('returns "staging" when NODE_ENV is "staging"', () => {
    process.env['NODE_ENV'] = 'staging';
    expect(resolveEnvironment()).toBe('staging');
  });

  it('returns "dev" when NODE_ENV is "dev"', () => {
    process.env['NODE_ENV'] = 'dev';
    expect(resolveEnvironment()).toBe('dev');
  });

  it('returns "dev" when NODE_ENV is undefined', () => {
    delete process.env['NODE_ENV'];
    expect(resolveEnvironment()).toBe('dev');
  });

  it('returns "dev" for unrecognized NODE_ENV values', () => {
    process.env['NODE_ENV'] = 'test';
    expect(resolveEnvironment()).toBe('dev');
  });
});
