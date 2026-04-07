#!/usr/bin/env node
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';

const stagedOutput = execFileSync(
  'git',
  ['diff', '--cached', '--name-only', '--diff-filter=ACMR', '-z'],
  { encoding: 'utf8' }
);

const stagedFiles = stagedOutput
  .split('\0')
  .filter(Boolean)
  .filter((file) => existsSync(file));

if (stagedFiles.length === 0) {
  process.exit(0);
}

const rustFiles = stagedFiles.filter((f) => f.endsWith('.rs'));
if (rustFiles.length > 0) {
  const fmtResult = spawnSync('rustfmt', ['--edition', '2024', ...rustFiles], {
    stdio: 'inherit',
  });

  if (fmtResult.error?.code === 'ENOENT') {
    console.warn('rustfmt not found, skipping Rust formatting');
  } else if (fmtResult.error) {
    throw fmtResult.error;
  } else if (fmtResult.status !== 0) {
    process.exit(fmtResult.status ?? 1);
  } else {
    execFileSync('git', ['add', '--', ...rustFiles]);
  }
}

const formatResult = spawnSync('nx', ['format:write', '--stdin'], {
  input: stagedFiles.join('\n'),
  stdio: ['pipe', 'inherit', 'inherit'],
});

if (formatResult.error) {
  throw formatResult.error;
}

if (formatResult.status !== 0) {
  process.exit(formatResult.status ?? 1);
}

const addResult = spawnSync('git', ['add', '--', ...stagedFiles], {
  stdio: 'inherit',
});

if (addResult.error) {
  throw addResult.error;
}

process.exit(addResult.status ?? 1);
