#!/bin/bash
# EXYONQ CODE INTEGRITY AUDITOR — canonical entrypoint.
# RULE_ID=EXYONQ-CODE-INTEGRITY-AUDITOR
#
# Usage:
#   ./scripts/integrity/exyonq-integrity-audit.sh              # full audit
#   ./scripts/integrity/exyonq-integrity-audit.sh --quick       # static fast checks
#   ./scripts/integrity/exyonq-integrity-audit.sh --changed     # focus on git changes
#   ./scripts/integrity/exyonq-integrity-audit.sh --selftest    # auditor regression fixtures
#   ./scripts/integrity/exyonq-integrity-audit.sh --out DIR     # override artifact dir
#
# Exit codes:
#   0 = PASS
#   1 = INTEGRITY_FINDINGS (CRITICAL or HIGH)
#   2 = AUDITOR_ERROR
#   3 = REVIEW_REQUIRED (REVIEW findings, no CRITICAL/HIGH)
#
# Artifacts (default):
#   .exyonq-local/integrity/<timestamp>/{summary.txt,findings.json,findings.tsv,evidence/}
#   .exyonq-local/integrity/latest/ → most recent run
#
# Principles:
#   CLAIM != EVIDENCE
#   CODE_PRESENT != FEATURE_WORKING
#   TEST_PRESENT != REAL_END_TO_END_BEHAVIOR
#   CONFIG_ACCEPTED != CONFIG_EFFECTIVE
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENGINE="$SCRIPT_DIR/lib/integrity_audit.py"
FIXTURES="$SCRIPT_DIR/fixtures"

pick_python() {
  # Absolute paths only — never invoke pyenv/homebrew shims here (can hang).
  if [[ -n "${EXYONQ_PYTHON3:-}" && -x "${EXYONQ_PYTHON3}" ]]; then
    printf '%s\n' "${EXYONQ_PYTHON3}"
    return 0
  fi
  if [[ -x /usr/bin/python3 ]]; then
    printf '%s\n' /usr/bin/python3
    return 0
  fi
  if [[ -x /opt/homebrew/bin/python3 ]]; then
    printf '%s\n' /opt/homebrew/bin/python3
    return 0
  fi
  return 1
}

usage() {
  sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'
}

MODE="full"
OUT=""
BASELINE="main"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --quick) MODE=quick; shift ;;
    --changed) MODE=changed; shift ;;
    --selftest) MODE=selftest; shift ;;
    --full) MODE=full; shift ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --baseline) BASELINE="${2:-main}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

PY="$(pick_python)" || { echo "ERROR: python3 not found" >&2; exit 2; }
[[ -f "$ENGINE" ]] || { echo "ERROR: missing engine $ENGINE" >&2; exit 2; }

# Preserve exit codes through the pipeline: never mask with grep/tee.
run_engine() {
  local rc=0
  if [[ "$MODE" == "selftest" ]]; then
    "$PY" "$ENGINE" selftest --fixtures "$FIXTURES" || rc=$?
  elif [[ -n "$OUT" ]]; then
    "$PY" "$ENGINE" "$MODE" --root "$REPO_ROOT" --out "$OUT" --baseline "$BASELINE" || rc=$?
  else
    "$PY" "$ENGINE" "$MODE" --root "$REPO_ROOT" --baseline "$BASELINE" || rc=$?
  fi
  return "$rc"
}

run_engine
exit $?
