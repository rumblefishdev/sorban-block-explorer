#!/usr/bin/env node
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';

const mode = process.argv[2];
const supportedModes = new Set(['staged', 'push']);
const targetCandidates = ['lint', 'typecheck', 'test'];

if (!supportedModes.has(mode)) {
  console.error('Expected mode to be one of: staged, push.');
  process.exit(1);
}

const run = (command, args, options = {}) => {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
    ...options,
  });

  if (result.error) {
    throw result.error;
  }

  return result;
};

const tryRun = (command, args) => {
  const result = run(command, args);
  if (result.status !== 0) {
    return null;
  }

  return result.stdout.trim();
};

const getStagedFiles = () =>
  execFileSync(
    'git',
    ['diff', '--cached', '--name-only', '--diff-filter=ACMR', '-z'],
    {
      encoding: 'utf8',
    }
  )
    .split('\0')
    .filter(Boolean)
    .filter((file) => existsSync(file));

const getAvailableTargets = () =>
  targetCandidates.filter((target) => {
    const output = execFileSync(
      'nx',
      ['show', 'projects', '--withTarget', target, '--json'],
      {
        encoding: 'utf8',
      }
    );
    const projects = JSON.parse(output);
    return Array.isArray(projects) && projects.length > 0;
  });

const refExists = (ref) =>
  run('git', ['rev-parse', '--verify', '--quiet', ref]).status === 0;

const resolveBaseRef = () => {
  const upstream = tryRun('git', [
    'rev-parse',
    '--abbrev-ref',
    '--symbolic-full-name',
    '@{upstream}',
  ]);
  if (upstream) {
    return upstream;
  }

  const originHead = tryRun('git', [
    'symbolic-ref',
    '--quiet',
    '--short',
    'refs/remotes/origin/HEAD',
  ]);
  if (originHead && refExists(originHead)) {
    return originHead;
  }

  for (const candidate of ['origin/master', 'origin/main', 'master', 'main']) {
    if (refExists(candidate)) {
      return candidate;
    }
  }

  const previousCommit = tryRun('git', [
    'rev-parse',
    '--verify',
    '--quiet',
    'HEAD~1',
  ]);
  if (previousCommit) {
    return previousCommit;
  }

  return 'HEAD';
};

const targets = getAvailableTargets();

if (targets.length === 0) {
  process.exit(0);
}

const nxArgs = ['affected', '-t', ...targets, '--outputStyle=static'];

if (mode === 'staged') {
  const stagedFiles = getStagedFiles();

  if (stagedFiles.length === 0) {
    process.exit(0);
  }

  const result = spawnSync('nx', [...nxArgs, '--stdin'], {
    input: stagedFiles.join('\n'),
    stdio: ['pipe', 'inherit', 'inherit'],
  });

  if (result.error) {
    throw result.error;
  }

  process.exit(result.status ?? 1);
}

const baseRef = resolveBaseRef();
console.log(`Running affected checks against ${baseRef}.`);

const result = spawnSync(
  'nx',
  [...nxArgs, `--base=${baseRef}`, '--head=HEAD'],
  {
    stdio: 'inherit',
  }
);

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
