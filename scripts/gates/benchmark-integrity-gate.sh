#!/usr/bin/env bash
# EXYONQ-BENCHMARK-INTEGRITY-ZERO-SHORTCUTS — product semantic shortcut gate.
# RULE_ID=EXYONQ-BENCHMARK-INTEGRITY-ZERO-SHORTCUTS
#
# Usage:
#   benchmark-integrity-gate.sh --selftest
#   benchmark-integrity-gate.sh --tree DIR
#   benchmark-integrity-gate.sh --audit DIR   # same as --tree; prints audit summary
#
# Exit 0 only when PRODUCT_SEMANTIC_BRANCH=0 and UNKNOWN=0 (or --selftest passes).
# Does not modify product code. Does not print "fixes".
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENGINE="$SCRIPT_DIR/lib/benchmark_integrity_scan.py"
FIXTURES="$SCRIPT_DIR/fixtures"

pick_python() {
  if [[ -n "${EXYONQ_PYTHON3:-}" && -x "${EXYONQ_PYTHON3}" ]]; then
    printf '%s\n' "$EXYONQ_PYTHON3"; return 0
  fi
  if [[ -x /usr/bin/python3 ]]; then printf '%s\n' /usr/bin/python3; return 0; fi
  if [[ -x /opt/homebrew/bin/python3 ]]; then printf '%s\n' /opt/homebrew/bin/python3; return 0; fi
  command -v python3
}

usage() {
  sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
}

MODE=""
ROOT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --selftest) MODE=selftest; shift ;;
    --tree|--audit) MODE=tree; ROOT="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$MODE" ]] || { echo "ERROR: mode required (--selftest|--tree DIR)" >&2; exit 2; }
PY="$(pick_python)"
[[ -f "$ENGINE" ]] || { echo "ERROR: missing $ENGINE" >&2; exit 2; }

case "$MODE" in
  selftest)
    exec "$PY" "$ENGINE" selftest --fixtures "$FIXTURES"
    ;;
  tree)
    [[ -n "$ROOT" ]] || ROOT="$REPO_ROOT"
    [[ -d "$ROOT" ]] || { echo "ERROR: not a directory: $ROOT" >&2; exit 2; }
    exec "$PY" "$ENGINE" tree --root "$ROOT"
    ;;
esac
