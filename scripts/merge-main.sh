#!/usr/bin/env bash
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
git fetch --quiet origin main
if git merge --no-edit origin/main; then
  exit 0
fi
git checkout --theirs .agents/skills/neomem-control/feature-graph.yaml 2>/dev/null || true
git add .agents/skills/neomem-control/feature-graph.yaml 2>/dev/null || true
git add -A proofs
remaining=$(git diff --name-only --diff-filter=U)
if [ -n "$remaining" ]; then
  echo "merge-main: resolve these by hand, then commit:" >&2
  echo "$remaining" >&2
  exit 1
fi
git commit --quiet --no-edit
echo "merge-main: proofs and the feature graph took main's side; run cargo xtask ci to re-prove before pushing"
