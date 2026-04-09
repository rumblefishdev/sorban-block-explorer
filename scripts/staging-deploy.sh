#!/usr/bin/env bash
set -euo pipefail

# Deploy to staging by creating and pushing a date-based git tag.
# Tag pattern: staging-YYYY.MM.DD-N (sequential N within the day).
# See ADR 0009 for rationale.
#
# Usage:
#   ./scripts/staging-deploy.sh              # tag current HEAD
#   ./scripts/staging-deploy.sh <commit-sha> # tag specific commit
#
# Rollback:
#   ./scripts/staging-deploy.sh <known-good-sha>

REMOTE="origin"
COMMIT="${1:-HEAD}"

# Warn if not on develop (unless tagging a specific commit)
if [ "$COMMIT" = "HEAD" ]; then
  BRANCH=$(git branch --show-current 2>/dev/null)
  BRANCH="${BRANCH:-detached}"
  if [ "$BRANCH" != "develop" ]; then
    echo "WARNING: you are on '${BRANCH}', not 'develop'."
    read -rp "Continue anyway? (y/N) " ans
    [ "$ans" = "y" ] || exit 1
  fi
fi

# Fetch tags from the same remote we push to
git fetch "$REMOTE" --tags --quiet

DATE=$(date +%Y.%m.%d)
MAX_N=$(git tag -l "staging-${DATE}-[0-9]*" | sed "s/staging-${DATE}-//" | grep -E '^[0-9]+$' | sort -n | tail -1)
N=$(( ${MAX_N:-0} + 1 ))
TAG="staging-${DATE}-${N}"

echo "Creating tag: ${TAG} → $(git rev-parse --short "$COMMIT")"
git tag "$TAG" "$COMMIT"
git push "$REMOTE" "$TAG"
echo "Pushed ${TAG} — staging deploy will start in GitHub Actions."
