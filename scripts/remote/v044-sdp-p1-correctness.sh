#!/usr/bin/env bash
# SDP-P1 correctness against a live ExyonQ (EXYONQ_SDP_P1=1).
# Usage: BASE=http://127.0.0.1:8080 bash scripts/remote/v044-sdp-p1-correctness.sh
set -euo pipefail
BASE="${BASE:-http://127.0.0.1:8080}"
PASS=0
FAIL=0
check() {
  local name=$1
  shift
  if "$@"; then
    echo "PASS $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL $name"
    FAIL=$((FAIL + 1))
  fi
}

check single_get curl -fsS -o /dev/null -w '%{http_code}' "$BASE/api/" | grep -q 200
check keepalive_100 bash -c "for i in \$(seq 1 100); do curl -fsS -o /dev/null --keepalive-time 60 \"$BASE/api/\" || exit 1; done"
# Smuggling-ish rejects (expect non-200 or connection error)
check te_reject bash -c "code=\$(curl -sS -o /dev/null -w '%{http_code}' -H 'Transfer-Encoding: chunked' \"$BASE/api/\" || true); [[ \$code != 200 ]]"
check dup_cl_reject bash -c "code=\$(curl -sS -o /dev/null -w '%{http_code}' -H 'Content-Length: 0' -H 'Content-Length: 0' \"$BASE/api/\" || true); [[ \$code != 200 ]]"

echo "PASS=$PASS FAIL=$FAIL"
[[ "$FAIL" -eq 0 ]]
