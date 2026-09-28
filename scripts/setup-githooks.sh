#!/usr/bin/env bash
# Install repo git hooks (pre-push: verify no private paths).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

git config core.hooksPath .githooks
chmod +x .githooks/pre-push
echo "Installed git hooks from .githooks/ (core.hooksPath=.githooks)"
