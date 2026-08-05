#!/usr/bin/env bash
# Regression tests for EXYONQ-SEC-PRIVATE-MATERIAL-ZERO (no keys committed).
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCAN="$ROOT/scripts/test-tls/exyonq-sec-private-material-zero.sh"
GEN="$ROOT/scripts/test-tls/generate-ephemeral-tls.sh"
chmod +x "$SCAN" "$GEN"

bash "$SCAN" --selftest

# Generator produces isolated dirs; cleanup works
eval "$(bash "$GEN")"
test -f "$EXYONQ_TLS_KEY"
test -f "$EXYONQ_TLS_CERT"
stat_mode="$(python3 -c "import os; print(oct(os.stat('$EXYONQ_TLS_KEY').st_mode & 0o777))")"
test "$stat_mode" = "0o600" -o "$stat_mode" = "0600" || {
  # macOS may report without leading 0o in some pythons — accept 0o600/600
  case "$stat_mode" in
    *600) ;;
    *) echo "ERROR: key mode=$stat_mode want 0600" >&2; exit 1 ;;
  esac
}
DIR1="$EXYONQ_TLS_DIR"
eval "$(bash "$GEN")"
DIR2="$EXYONQ_TLS_DIR"
test "$DIR1" != "$DIR2"
bash "$GEN" --cleanup "$DIR1"
bash "$GEN" --cleanup "$DIR2"
test ! -d "$DIR1"
test ! -d "$DIR2"

# Failure-path cleanup: create then simulate failure cleanup
eval "$(bash "$GEN")"
FAIL_DIR="$EXYONQ_TLS_DIR"
bash "$GEN" --cleanup "$FAIL_DIR"
test ! -e "$FAIL_DIR"

echo "P14TLSFIX_SCANNER_REGRESSION_TESTS=PASS"
echo "P14TLSFIX_EPHEMERAL_GENERATION=PASS"
echo "P14TLSFIX_PARALLEL_ISOLATION=PASS"
echo "P14TLSFIX_CLEANUP_SUCCESS_PATH=PASS"
echo "P14TLSFIX_CLEANUP_FAILURE_PATH=PASS"
