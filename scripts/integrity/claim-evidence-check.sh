#!/usr/bin/env bash
# CLAIM-EVIDENCE-INTEGRITY deterministic checker entrypoint.
# RULE_ID=CLAIM-EVIDENCE-INTEGRITY
#
# Usage:
#   claim-evidence-check.sh --check PATH [PATH...]   # file or directory
#   claim-evidence-check.sh --selftest               # independent case verifier
#
# Exit codes:
#   0 CEI_CHECK=PASS      no mechanically provable mismatch (not "claim proven")
#   1 CEI_CHECK=FAIL      proven claim/evidence mismatch
#   2 CHECKER_ERROR       unparseable input or engine failure
#   3 CEI_CHECK=REVIEW    cannot decide mechanically — never guessed as PASS
#
# Read-only. Does not modify product code, tests, or gates.
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENGINE="$SCRIPT_DIR/lib/claim_evidence_check.py"
VERIFIER="$SCRIPT_DIR/tests/verify-claim-evidence-cases.sh"

pick_python() {
  if [[ -n "${EXYONQ_PYTHON3:-}" && -x "${EXYONQ_PYTHON3}" ]]; then
    printf '%s\n' "$EXYONQ_PYTHON3"; return 0
  fi
  if [[ -x /usr/bin/python3 ]]; then printf '%s\n' /usr/bin/python3; return 0; fi
  if [[ -x /opt/homebrew/bin/python3 ]]; then printf '%s\n' /opt/homebrew/bin/python3; return 0; fi
  command -v python3
}

usage() { sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; }

[[ $# -gt 0 ]] || { echo "ERROR: mode required (--check PATH|--selftest)" >&2; usage >&2; exit 2; }

case "$1" in
  --selftest)
    [[ -x "$VERIFIER" || -f "$VERIFIER" ]] || { echo "ERROR: missing $VERIFIER" >&2; exit 2; }
    exec /bin/bash "$VERIFIER"
    ;;
  --check)
    shift
    [[ $# -gt 0 ]] || { echo "ERROR: --check requires at least one path" >&2; exit 2; }
    [[ -f "$ENGINE" ]] || { echo "ERROR: missing $ENGINE" >&2; exit 2; }
    exec "$(pick_python)" "$ENGINE" check "$@"
    ;;
  -h|--help)
    usage; exit 0
    ;;
  *)
    echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2
    ;;
esac
