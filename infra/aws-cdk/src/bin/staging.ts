#!/usr/bin/env node
import { createRequire } from 'node:module';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import type { EnvironmentConfig } from '../lib/types.js';
import { createApp } from '../lib/app.js';

const require = createRequire(import.meta.url);
const params = require('../../envs/staging.json') as EnvironmentConfig;

// Repo root: this file is at dist/bin/staging.js inside infra/aws-cdk/
const repoRoot = resolve(
  dirname(fileURLToPath(import.meta.url)),
  '..',
  '..',
  '..',
  '..'
);

createApp({ config: params, cargoWorkspacePath: repoRoot });
