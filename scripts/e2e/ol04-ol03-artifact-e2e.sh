#!/usr/bin/env bash
# OL-04 and OL-03 real E2E bound to one artifact hash.
#
# Order: record hash -> unit tests -> verify hash -> OL-04 E2E -> verify hash
# and log hash -> OL-03 E2E -> verify hash and log hash -> SHA256SUMS.
# A binary replaced by a rebuild between any two stages fails the run with
# ARTIFACT_DRIFT instead of producing evidence against the wrong artifact.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
COMPAT="${EXYONQ_COMPAT_BIN:-$ROOT/target/debug/exyonq-compat}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="${OL04_OL03_RUN_DIR:-$ROOT/.exyonq-local/evidence/ol04-ol03-$STAMP}"
GUARD="$ROOT/scripts/gates/artifact-integrity.sh"
STATE="$RUN/artifact-integrity.tsv"
FAIL=0

mkdir -p "$RUN"
: >"$STATE"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

echo "== environment ==" | tee "$RUN/environment.txt"
{
  echo "host=$(uname -s)/$(uname -m)"
  echo "kernel=$(uname -r)"
  echo "binary=$BIN"
  echo "compat=$COMPAT"
  echo "run=$RUN"
  echo "utc=$STAMP"
} | tee -a "$RUN/environment.txt"
"$BIN" --version | tee -a "$RUN/environment.txt"
"$COMPAT" --version 2>/dev/null | head -5 | tee -a "$RUN/environment.txt"

"$GUARD" record "$STATE" exyonq "$BIN" | tee -a "$RUN/integrity.log" || FAIL=1
"$GUARD" record "$STATE" exyonq-compat "$COMPAT" | tee -a "$RUN/integrity.log" || FAIL=1

run_test() {
  local name=$1
  shift
  echo "== TEST $name =="
  if "$@" >"$RUN/test-$name.log" 2>&1; then
    echo "TEST_PASS $name log=$RUN/test-$name.log"
  else
    local rc=$?
    echo "TEST_FAIL $name ec=$rc log=$RUN/test-$name.log"
    tail -25 "$RUN/test-$name.log"
    FAIL=1
  fi
}

run_test waf-challenge cargo test -p exyonq-waf --lib --locked challenge
run_test core-captcha cargo test -p exyonq-core --lib --locked captcha
run_test ir-waf cargo test -p exyonq-config-ir --lib --locked waf
run_test core-client-throttle cargo test -p exyonq-core --lib --locked client_throttle
# The OpenLiteSpeed importer has no unit-test target; its contract (translation,
# validation and live throttling) is exercised by the OL-03 E2E below.

echo "== integrity after tests =="
"$GUARD" verify "$STATE" exyonq "$BIN" | tee -a "$RUN/integrity.log" || FAIL=1
"$GUARD" verify "$STATE" exyonq-compat "$COMPAT" | tee -a "$RUN/integrity.log" || FAIL=1

echo "== E2E OL-04 =="
if OL04_RUN_DIR="$RUN/ol04" EXYONQ_BIN="$BIN" \
  bash "$ROOT/scripts/e2e/ol04-challenge-e2e.sh" >"$RUN/ol04-console.log" 2>&1; then
  echo "E2E_PASS OL-04 log=$RUN/ol04-console.log"
else
  rc=$?
  echo "E2E_FAIL OL-04 ec=$rc log=$RUN/ol04-console.log"
  tail -40 "$RUN/ol04-console.log"
  FAIL=1
fi
grep -E '^(SUMMARY|RESULT=)' "$RUN/ol04-console.log" || true

echo "== integrity after OL-04 =="
"$GUARD" verify "$STATE" exyonq "$BIN" | tee -a "$RUN/integrity.log" || FAIL=1
"$GUARD" verify "$STATE" exyonq-compat "$COMPAT" | tee -a "$RUN/integrity.log" || FAIL=1
"$GUARD" log "$STATE" exyonq "$RUN/ol04-console.log" | tee -a "$RUN/integrity.log" || FAIL=1

echo "== E2E OL-03 =="
if OL03_RUN_DIR="$RUN/ol03" EXYONQ_BIN="$BIN" EXYONQ_COMPAT_BIN="$COMPAT" \
  bash "$ROOT/scripts/e2e/ol03-full-e2e.sh" >"$RUN/ol03-console.log" 2>&1; then
  echo "E2E_PASS OL-03 log=$RUN/ol03-console.log"
else
  rc=$?
  echo "E2E_FAIL OL-03 ec=$rc log=$RUN/ol03-console.log"
  tail -40 "$RUN/ol03-console.log"
  FAIL=1
fi
grep -E '^(SUMMARY|RESULT=)' "$RUN/ol03-console.log" || true

echo "== integrity after OL-03 =="
"$GUARD" verify "$STATE" exyonq "$BIN" | tee -a "$RUN/integrity.log" || FAIL=1
"$GUARD" verify "$STATE" exyonq-compat "$COMPAT" | tee -a "$RUN/integrity.log" || FAIL=1
"$GUARD" log "$STATE" exyonq "$RUN/ol03-console.log" | tee -a "$RUN/integrity.log" || FAIL=1

{
  printf '%s  %s\n' "$(hash_file "$BIN")" "exyonq ($(uname -m), built on this host)"
  printf '%s  %s\n' "$(hash_file "$COMPAT")" "exyonq-compat ($(uname -m), built on this host)"
  (cd "$RUN" && sha256sum ./*.log ./*.txt ./*.tsv 2>/dev/null)
} >"$RUN/SHA256SUMS"

echo
cat "$RUN/SHA256SUMS"
echo
if [[ $FAIL -eq 0 ]]; then
  echo "RESULT=PASS OL04 OL03 ARTIFACT_BOUND run=$RUN"
  exit 0
fi
echo "RESULT=FAIL OL04 OL03 ARTIFACT_BOUND run=$RUN"
exit 1
