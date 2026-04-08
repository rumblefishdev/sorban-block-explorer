#!/usr/bin/env node
import { createRequire } from 'node:module';

import type { CicdConfig } from '../lib/types.js';
import { createCicdApp } from '../lib/cicd-app.js';

const require = createRequire(import.meta.url);
const config = require('../../envs/cicd.json') as CicdConfig;

createCicdApp(config);
