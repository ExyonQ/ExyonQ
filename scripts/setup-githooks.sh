#!/usr/bin/env bash
# Install repo git hooks (pre-commit: staged index; pre-push: publication set).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for hook in pre-commit pre-push; do
  [[ -f ".githooks/$hook" ]] || { echo "ERROR: missing .githooks/$hook" >&2; exit 1; }
  chmod +x ".githooks/$hook"
done
git config core.hooksPath .githooks
echo "Installed git hooks from .githooks/ (core.hooksPath=.githooks)"
