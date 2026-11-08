#!/usr/bin/env bash
# L-02: production routing, timeout and URI decisions must not be keyed by
# benchmark path literals (`/api`, `/site`) or by a route named `site`.
#
# Usage:
#   scripts/gates/l02-no-benchmark-path-decisions.sh
#   scripts/gates/l02-no-benchmark-path-decisions.sh --root DIR
#   scripts/gates/l02-no-benchmark-path-decisions.sh --selftest
#
# The scanner reads each decision file completely and masks only test regions
# (`#[cfg(test)]` items, test files and test-only modules). Non-decision API
# surfaces are outside the decision roots and are classified in the increment
# report.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENGINE="$SCRIPT_DIR/lib/l02_scan.py"
FIXTURES="$SCRIPT_DIR/fixtures/l02"

MODE=scan
ROOT="$REPO_ROOT"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --root)
      MODE=scan
      ROOT="${2:-}"
      shift 2
      ;;
    --selftest)
      MODE=selftest
      shift
      ;;
    -h|--help)
      sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ ! -f "$ENGINE" ]]; then
  echo "ERROR: missing scanner: $ENGINE" >&2
  exit 2
fi

case "$MODE" in
  scan)
    [[ -n "$ROOT" ]] || { echo "ERROR: --root requires a directory" >&2; exit 2; }
    exec python3 "$ENGINE" scan --root "$ROOT"
    ;;
  selftest)
    exec python3 "$ENGINE" selftest --fixtures "$FIXTURES"
    ;;
esac
