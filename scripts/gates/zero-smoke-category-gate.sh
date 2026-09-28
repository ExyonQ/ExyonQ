#!/usr/bin/env bash
# ZERO_SMOKE_CATEGORY_GATE — fail closed if smoke testing category returns.
# Regression protection for ZF-001 (E2E must not live under / delegate to smoke/).
#
# Usage:
#   scripts/gates/zero-smoke-category-gate.sh --selftest
#   scripts/gates/zero-smoke-category-gate.sh --tree <dir>
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE="${1:-}"

usage() {
  cat <<'EOF'
Usage:
  scripts/gates/zero-smoke-category-gate.sh --selftest
  scripts/gates/zero-smoke-category-gate.sh --tree <dir>
EOF
}

scan_tree() {
  local tree="$1"
  local fail=0

  if [[ -d "$tree/scripts/smoke" ]]; then
    echo "FAIL: scripts/smoke/ directory exists under $tree"
    fail=1
  fi

  # Executable/test scripts must not be named *smoke* under scripts/
  while IFS= read -r f; do
    echo "FAIL: smoke-named script path: $f"
    fail=1
  done < <(find "$tree/scripts" \( -name '*smoke*' -o -path '*/smoke/*' \) \
    \( -type f -o -type d \) 2>/dev/null | grep -v '/gates/no-smoke-as-proof-gate.sh$' \
    | grep -v '/gates/zero-smoke-category-gate.sh$' || true)

  # E2E entrypoints must not delegate into smoke/
  if [[ -d "$tree/scripts/e2e" ]]; then
    while IFS= read -r line; do
      echo "FAIL: e2e delegates to smoke: $line"
      fail=1
    done < <(rg -n 'scripts/smoke/|exec .*smoke/|source .*smoke/' \
      "$tree/scripts/e2e" --glob '*.sh' 2>/dev/null || true)
  fi

  if [[ "$fail" -ne 0 ]]; then
    echo "ZERO_SMOKE_CATEGORY_GATE = FAIL"
    return 1
  fi
  echo "ZERO_SMOKE_CATEGORY_GATE = PASS"
  return 0
}

selftest() {
  # Non-local path so EXIT trap under `set -u` can always see it.
  ZERO_SMOKE_SELFTEST_TMP="$(mktemp -d "${TMPDIR:-/tmp}/zero-smoke-gate.XXXXXX")"
  cleanup_selftest() { rm -rf "${ZERO_SMOKE_SELFTEST_TMP:-}"; }
  trap cleanup_selftest EXIT

  mkdir -p "$ZERO_SMOKE_SELFTEST_TMP/must-pass/scripts/e2e"
  printf '#!/bin/bash\necho ok\n' >"$ZERO_SMOKE_SELFTEST_TMP/must-pass/scripts/e2e/static-e2e.sh"

  mkdir -p "$ZERO_SMOKE_SELFTEST_TMP/must-fail-dir/scripts/smoke"
  printf '#!/bin/bash\necho bad\n' >"$ZERO_SMOKE_SELFTEST_TMP/must-fail-dir/scripts/smoke/x.sh"

  mkdir -p "$ZERO_SMOKE_SELFTEST_TMP/must-fail-delegate/scripts/e2e"
  printf '#!/bin/bash\nexec bash scripts/smoke/old.sh\n' \
    >"$ZERO_SMOKE_SELFTEST_TMP/must-fail-delegate/scripts/e2e/proxy-e2e.sh"

  if ! scan_tree "$ZERO_SMOKE_SELFTEST_TMP/must-pass" >/tmp/zero-smoke-pass.out 2>&1; then
    echo "SELFTEST FAIL: must-pass rejected"
    cat /tmp/zero-smoke-pass.out || true
    return 1
  fi
  if scan_tree "$ZERO_SMOKE_SELFTEST_TMP/must-fail-dir" >/tmp/zero-smoke-fail-dir.out 2>&1; then
    echo "SELFTEST FAIL: must-fail-dir accepted"
    cat /tmp/zero-smoke-fail-dir.out || true
    return 1
  fi
  if scan_tree "$ZERO_SMOKE_SELFTEST_TMP/must-fail-delegate" >/tmp/zero-smoke-fail-del.out 2>&1; then
    echo "SELFTEST FAIL: must-fail-delegate accepted"
    cat /tmp/zero-smoke-fail-del.out || true
    return 1
  fi
  echo "ZERO_SMOKE_CATEGORY_GATE_SELFTEST = PASS"
  return 0
}

case "$MODE" in
  --selftest) selftest ;;
  --tree)
    TREE="${2:-}"
    [[ -n "$TREE" ]] || { usage >&2; exit 2; }
    scan_tree "$TREE"
    ;;
  *) usage >&2; exit 2 ;;
esac
