#!/usr/bin/env bash
set -euo pipefail
gh auth refresh --hostname github.com --scopes workflow
cd "$(git rev-parse --show-toplevel)"
mkdir -p .github/workflows
git mv ci/workflows/ci.yml .github/workflows/ci.yml
git commit -m "ci: enable the GitHub Actions workflow"
git push
