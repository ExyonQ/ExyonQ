#!/usr/bin/env bash
# Regression guard for the publication scanner mode.
#
# A future workflow, release entrypoint or shell helper must not silently go
# back to a mode that is blind to untracked files or to staged blobs. The test
# scans the real publication surfaces and also self-tests its detection logic
# against a temporary weak workflow.
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
ROOT_OVERRIDE=""
if [[ "${1:-}" == "--root" ]]; then
  [[ -n "${2:-}" ]] || { echo "ERROR: --root requires a directory" >&2; exit 2; }
  ROOT="$(cd "$2" && pwd)"
  ROOT_OVERRIDE=1
fi

WEAK_MODE="--git-tree"
ALLOWLIST=(
  "scripts/security/scan-private-material.sh"
  "scripts/security/lib/private_material_scan.py"
  "scripts/security/tests/test-private-material-scanner.sh"
  "scripts/security/tests/test-publication-path-guard.sh"
  "docs/security/audit-v0.9.0.md"
  "docs/security/audit-v0.4.3.md"
)

is_allowlisted() {
  local rel="$1" allowed
  for allowed in "${ALLOWLIST[@]}"; do
    [[ "$rel" == "$allowed" ]] && return 0
  done
  return 1
}

FILES=()
if [[ -d "$ROOT/.github/workflows" ]]; then
  while IFS= read -r f; do FILES+=("$f"); done < <(
    find "$ROOT/.github/workflows" -type f \( -name '*.yml' -o -name '*.yaml' \) -print | sort
  )
fi
if [[ -d "$ROOT/scripts" ]]; then
  while IFS= read -r f; do FILES+=("$f"); done < <(
    find "$ROOT/scripts" -type f \( -name '*.sh' -o -name '*.py' \) -print | sort
  )
fi

# Current documentation that carries copy/paste publication commands. Historical
# audits are allowlisted above and are not rewritten.
for doc in \
  "docs/security/private-material-zero.md" \
  "docs/releases/0.9.1-requirements-evidence.md" \
  "docs/architecture/070-ol03-client-throttle.md" \
  "docs/governance/evidence/INDEX.md" \
  "AGENTS.md"; do
  if [[ -f "$ROOT/$doc" ]]; then
    FILES+=("$ROOT/$doc")
  fi
done

echo "PUBLICATION_PATH_GUARD_FILES=${#FILES[@]}"
fail=0
for f in "${FILES[@]}"; do
  rel="${f#"$ROOT"/}"
  is_allowlisted "$rel" && continue
  if hits="$(grep -nF -- "$WEAK_MODE" "$f" 2>/dev/null)"; then
    echo "ERROR: weak scanner mode in publication path: $rel" >&2
    printf '%s\n' "$hits" >&2
    fail=1
  fi
done

if [[ -z "$ROOT_OVERRIDE" ]]; then
  # The canonical automation must call the complete publication mode.
  for f in \
    ".github/workflows/ci.yml" \
    ".github/workflows/security.yml" \
    ".github/workflows/release.yml" \
    "scripts/security/private-material-release-gates.sh"; do
    if [[ -f "$ROOT/$f" ]] && ! grep -qF -- "--publication" "$ROOT/$f"; then
      echo "ERROR: publication mode missing from $f" >&2
      fail=1
    fi
  done
  for f in ".githooks/pre-commit" ".githooks/pre-push"; do
    if [[ ! -x "$ROOT/$f" ]]; then
      echo "ERROR: required executable hook missing: $f" >&2
      fail=1
    fi
  done
  if ! grep -qF -- "--git-index" "$ROOT/.githooks/pre-commit"; then
    echo "ERROR: pre-commit does not scan the staged index" >&2
    fail=1
  fi
  if ! grep -qF -- "--publication" "$ROOT/.githooks/pre-push"; then
    echo "ERROR: pre-push does not scan the complete publication set" >&2
    fail=1
  fi
else
  # Self-test: this mode must reject a weak workflow and accept a complete one.
  weak="$ROOT/.github/workflows/weak-self-test.yml"
  mkdir -p "$ROOT/.github/workflows"
  printf '%s\n' \
    'name: weak-self-test' \
    'on: [push]' \
    'jobs:' \
    '  scan:' \
    '    runs-on: ubuntu-latest' \
    '    steps:' \
    '      - run: bash scripts/security/scan-private-material.sh --git-tree --repo .' \
    >"$weak"
  if bash "$0" --root "$ROOT" >/dev/null 2>&1; then
    echo "ERROR: guard self-test did not reject $WEAK_MODE" >&2
    fail=1
  else
    echo "PUBLICATION_PATH_GUARD_SELFTEST_REJECT=PASS"
  fi
  printf '%s\n' \
    'name: clean-self-test' \
    'on: [push]' \
    'jobs:' \
    '  scan:' \
    '    runs-on: ubuntu-latest' \
    '    steps:' \
    '      - run: bash scripts/security/scan-private-material.sh --publication --repo .' \
    >"$weak"
  if ! bash "$0" --root "$ROOT" >/dev/null 2>&1; then
    echo "ERROR: guard self-test rejected a complete publication invocation" >&2
    fail=1
  else
    echo "PUBLICATION_PATH_GUARD_SELFTEST_ACCEPT=PASS"
  fi
  rm -f "$weak"
fi

if [[ "$fail" -ne 0 ]]; then
  echo "PUBLICATION_PATH_GUARD=FAIL" >&2
  exit 1
fi
echo "PUBLICATION_PATH_GUARD=PASS"
