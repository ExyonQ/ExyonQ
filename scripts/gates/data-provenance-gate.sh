#!/usr/bin/env bash
# DATA provenance gate — PROJECT-INTEGRITY-NO-SHORTCUTS-NO-FAKE-DATA.
# RULE_ID=PROJECT-INTEGRITY-NO-SHORTCUTS-NO-FAKE-DATA
#
# Usage:
#   data-provenance-gate.sh --selftest
#   data-provenance-gate.sh --check RESULT_DIR
#
# Does not fabricate metrics. Does not modify product code.
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENGINE="$SCRIPT_DIR/lib/data_provenance_scan.py"
FIXTURES="$SCRIPT_DIR/fixtures/data-provenance"

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
PATH_ARG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --selftest) MODE=selftest; shift ;;
    --check) MODE=check; PATH_ARG="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$MODE" ]] || { echo "ERROR: mode required (--selftest|--check DIR)" >&2; exit 2; }
PY="$(pick_python)"
[[ -f "$ENGINE" ]] || { echo "ERROR: missing $ENGINE" >&2; exit 2; }

case "$MODE" in
  selftest)
    exec "$PY" "$ENGINE" selftest --fixtures "$FIXTURES"
    ;;
  check)
    [[ -n "$PATH_ARG" ]] || { echo "ERROR: --check requires DIR" >&2; exit 2; }
    exec "$PY" "$ENGINE" check --path "$PATH_ARG"
    ;;
esac
