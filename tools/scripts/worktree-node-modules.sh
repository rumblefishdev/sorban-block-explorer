#!/usr/bin/env sh
# Provision node_modules for a git worktree.
#
# Why this exists: npm workspaces materialise `node_modules/@rumblefish/*` as
# symlinks RELATIVE to node_modules' own location (`../../libs/ui`, …). A
# worktree whose node_modules is a symlink to the main checkout therefore
# resolves every workspace package to MAIN's source tree — so `web` typechecks
# against whatever branch main happens to be parked on, and husky's pre-commit
# hook fails with errors that exist in no file you touched.
#
# The fix is a real directory, cloned from main's. On APFS `cp -c` is a
# copy-on-write clone: ~20 s, ~30 MB of directory metadata, no shared blocks
# to write through, and the relative workspace symlinks now resolve LOCALLY.
#
# Run from anywhere inside the worktree. Idempotent.
set -e

ROOT=$(git rev-parse --show-toplevel)
MAIN=$(git worktree list --porcelain | awk '/^worktree /{print $2; exit}')
cd "$ROOT"

if [ "$ROOT" = "$MAIN" ]; then
  echo "main checkout — nothing to provision (use 'npm ci' here)"
  exit 0
fi

# A symlinked node_modules is the broken shape this script exists to replace.
if [ -L node_modules ]; then
  echo "replacing symlinked node_modules (resolves workspaces to main's tree)"
  mv node_modules "$MAIN/.trash/node_modules.symlink.$(basename "$ROOT").$$"
fi

if [ -d node_modules ] && [ ! -L node_modules ]; then
  echo "node_modules already a real directory"
elif cmp -s package-lock.json "$MAIN/package-lock.json" && [ -d "$MAIN/node_modules" ]; then
  echo "cloning main's node_modules (same lockfile)…"
  # ponytail: APFS clonefile. Falls back to a full `npm ci` off APFS, which is
  # correct but slow — swap in `cp -al` if a non-APFS volume ever matters.
  cp -Rpc "$MAIN/node_modules" node_modules || npm ci
else
  echo "lockfile differs from main's (or main has no node_modules) — npm ci"
  npm ci
fi

# The check: a workspace package MUST resolve inside this worktree. If it
# resolves into main, the layout is the broken one and every typecheck lies.
probe=node_modules/@rumblefish/soroban-block-explorer-ui
real=$(cd "$probe" 2>/dev/null && pwd -P) || {
  echo "FAIL: $probe does not resolve" >&2; exit 1
}
case "$real" in
  "$ROOT"/*) echo "ok: @rumblefish/* resolves to $real" ;;
  *) echo "FAIL: $probe resolves OUTSIDE this worktree -> $real" >&2; exit 1 ;;
esac
