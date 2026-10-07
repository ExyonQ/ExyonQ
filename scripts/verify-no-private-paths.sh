#!/usr/bin/env bash
# Fail CI / pre-push if private paths or local absolute paths are tracked by git.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PRIVATE_PREFIXES=(
  inspiracion/
  graphify-out/
  docs/product/
  benchmarks/caos-tests/
  benchmark-dev/
  docs/benchmarks-dev/
  # Publication boundary: lab measurement and the benchmark mock stay in the
  # lab tree. CI and pre-push fail if any of these paths are tracked again.
  scripts/remote/
  scripts/r3/
  scripts/reality/
  scripts/smoke/
  scripts/soak/
  scripts/allocator/
  scripts/architecture/
  scripts/integrity/
  scripts/fault/
  scripts/operations/
  scripts/dev/
  tests/integration/
  tools/r3-p4-mock/
)

fail=0

# .cursor/ is mostly local IDE config; only team-shared skills (*.md) + routing rule may be tracked.
while IFS= read -r path; do
  [[ -z "$path" ]] && continue
  case "$path" in
    .cursor/skills/*.md|.cursor/rules/122-exyonq-pr-skills.mdc) continue ;;
    *)
      echo "ERROR: private path tracked by git: $path"
      fail=1
      ;;
  esac
done < <(git ls-files '.cursor/' 2>/dev/null || true)

for prefix in "${PRIVATE_PREFIXES[@]}"; do
  while IFS= read -r path; do
    [[ -z "$path" ]] && continue
    echo "ERROR: private path tracked by git: $path"
    fail=1
  done < <(git ls-files "$prefix" 2>/dev/null || true)
done

while IFS= read -r path; do
  [[ -z "$path" ]] && continue
  echo "ERROR: private path tracked by git: $path"
  fail=1
done < <(git ls-files 'docs/evaluacion-*.md' 2>/dev/null || true)

ABSOLUTE_LOCAL_PATH_RE='(/Users/[^/]+/|/Volumes/[^/]+/)'

while IFS= read -r path; do
  [[ -z "$path" ]] && continue
  echo "ERROR: tracked file contains absolute local path: $path"
  fail=1
done < <(
  git grep -l -E "$ABSOLUTE_LOCAL_PATH_RE" -- benchmarks docs .cursor/skills .cursor/rules/122-exyonq-pr-skills.mdc 2>/dev/null || true
)

if [[ "$fail" -ne 0 ]]; then
  echo "Remove with: git rm -r --cached <path> && commit"
  echo "For absolute paths: use repo-relative paths instead of /Users/... or /Volumes/..."
  exit 1
fi

echo "OK: no private paths in git index"
