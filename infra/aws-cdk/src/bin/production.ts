#!/usr/bin/env node
import { createRequire } from 'node:module';

import type { EnvironmentConfig } from '../lib/types.js';
import { createApp } from '../lib/app.js';

const require = createRequire(import.meta.url);
const params = require('../../envs/production.json') as EnvironmentConfig;

createApp(params);
