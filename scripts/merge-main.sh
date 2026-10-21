#!/usr/bin/env bash
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
git fetch --quiet origin main
graph=.agents/skills/neomem-control/feature-graph.yaml
proven_here=$(git show HEAD:$graph | awk '/^- id:/{id=$3} /^  status: proven/{print id}' | sort)
proven_on_main=$(git show origin/main:$graph | awk '/^- id:/{id=$3} /^  status: proven/{print id}' | sort)
reprove=$(comm -23 <(echo "$proven_here") <(echo "$proven_on_main") | tr '\n' ' ')
if git merge --no-edit origin/main; then
  exit 0
fi
git checkout --theirs .agents/skills/neomem-control/feature-graph.yaml 2>/dev/null || true
git add .agents/skills/neomem-control/feature-graph.yaml 2>/dev/null || true
git add -A -- proofs
git restore --source=origin/main --staged --worktree -- proofs
remaining=$(git diff --name-only --diff-filter=U)
if [ -n "$remaining" ]; then
  echo "merge-main: resolve these by hand, then commit:" >&2
  echo "$remaining" >&2
  exit 1
fi
git commit --quiet --no-edit
echo "merge-main: proofs and the feature graph took main's side; run cargo xtask ci to re-prove before pushing"
if [ -n "${reprove// }" ]; then
  echo "merge-main: also re-prove what this branch had proven: neomemctl prove <id> for: $reprove"
fi
