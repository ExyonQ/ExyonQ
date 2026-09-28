#!/usr/bin/env bash
# Oracle / Plan-03 baseline evidence guard — harness remediation hygiene.
#
# Usage:
#   bash scripts/architecture/verify-oracle-baseline-evidence.sh [LOG]
#   LOG defaults to newest benchmarks/performance-contract/results/baseline-run-*.log
#
# Exit 0 = log semantics consistent. Exit 1 = evidence misrepresentation risk.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

LOG="${1:-}"
if [[ -z "$LOG" ]]; then
  LOG="$(ls -t "$ROOT"/benchmarks/performance-contract/results/baseline-run-*.log 2>/dev/null | head -1 || true)"
fi

fail() {
  echo "verify-oracle-baseline-evidence: FAIL: $*" >&2
  exit 1
}

pass() {
  echo "verify-oracle-baseline-evidence: PASS: $*"
}

if [[ -z "$LOG" || ! -f "$LOG" ]]; then
  pass "no baseline-run log to audit (skip)"
  exit 0
fi

if ! grep -q '^DONE ' "$LOG"; then
  fail "log missing DONE line — ORACLE_RUN_LIFECYCLE=INVALID_TERMINATION ($LOG)"
fi

DONE_LINE="$(grep '^DONE ' "$LOG" | tail -1)"
QUICK_RC="$(echo "$DONE_LINE" | sed -n 's/.*quick_rc=\([0-9]*\).*/\1/p')"
FULL_RC="$(echo "$DONE_LINE" | sed -n 's/.*full_rc=\([0-9]*\).*/\1/p')"
POST_RC="$(echo "$DONE_LINE" | sed -n 's/.*post_rc=\([0-9]*\).*/\1/p')"
SEED_RC="$(echo "$DONE_LINE" | sed -n 's/.*seed_rc=\([0-9]*\).*/\1/p')"
CLEANUP_RC="$(echo "$DONE_LINE" | sed -n 's/.*cleanup_rc=\([0-9]*\).*/\1/p')"
FINAL_RC="$(echo "$DONE_LINE" | sed -n 's/.*final_rc=\([0-9]*\).*/\1/p')"
TERM_STATUS="$(echo "$DONE_LINE" | sed -n 's/.*termination_status=\([^ ]*\).*/\1/p')"

for field in QUICK_RC FULL_RC POST_RC FINAL_RC TERM_STATUS; do
  if [[ -z "${!field}" ]]; then
    fail "DONE line missing required metadata ($field): $DONE_LINE"
  fi
done

# Documented exit codes (benchmarks/performance-contract/README.md)
# 0 PASS/WARNING  1 REGRESSION_FAIL  2 INVALID_RESULT  3 NOT_COMPARABLE
if grep -q '^baseline_status= accepted' "$LOG" && [[ "$FULL_RC" != "0" ]]; then
  fail "baseline_status=accepted does NOT imply full compare PASS (full_rc=$FULL_RC). ORACLE_EVIDENCE=DIAGNOSTIC_ONLY"
fi

if [[ "$CLEANUP_RC" != "0" ]]; then
  fail "cleanup_rc=$CLEANUP_RC — teardown incomplete"
fi

if [[ "$FINAL_RC" == "0" && ( "$QUICK_RC" != "0" || "$FULL_RC" != "0" ) ]]; then
  fail "final_rc=0 inconsistent with gate failures (quick_rc=$QUICK_RC full_rc=$FULL_RC)"
fi

if [[ "$FINAL_RC" != "0" && "$TERM_STATUS" == "CLEAN_EXIT" ]]; then
  fail "termination_status=CLEAN_EXIT but final_rc=$FINAL_RC"
fi

# Collect-only must not masquerade as competitive PASS in JSON sidecars referenced in log.
for json_path in $(grep -oE 'quick-real-[0-9TZ]+\.json|full-real-[0-9TZ]+\.json|post-accept-[0-9TZ]+\.json' "$LOG" 2>/dev/null | sort -u); do
  full_json="$ROOT/benchmarks/performance-contract/results/$json_path"
  if [[ -f "$full_json" ]]; then
    python3 - "$full_json" <<'PY' || fail "JSON sidecar invalid: $full_json"
import json, sys
path = sys.argv[1]
doc = json.load(open(path))
verdict = doc.get("overall_verdict", "")
scenarios = doc.get("scenarios", [])
mode = doc.get("mode")
if verdict == "PASS" and len(scenarios) == 0:
    raise SystemExit("PASS with empty scenarios")
if mode == "collect_only" and verdict == "PASS":
    raise SystemExit("collect_only presented as PASS")
if mode == "collect_only" and doc.get("collection_status") != "VALID" and verdict == "COLLECT_ONLY":
    pass
PY
  fi
done

# quick/full directory prefix hygiene
if grep -q 'full_dir=.*perf-contract-quick-' "$LOG"; then
  fail "full_dir uses perf-contract-quick prefix (profile path mix)"
fi
if grep -q 'quick_dir=.*perf-contract-full-' "$LOG"; then
  fail "quick_dir uses perf-contract-full prefix (profile path mix)"
fi

if grep -q 'INVALID_SAMPLES:' "$LOG"; then
  fail "incomplete samples detected in log"
fi

# Wrapper must not be cited as official evidence when any gate leg failed.
if [[ "$QUICK_RC" != "0" || "$FULL_RC" != "0" ]]; then
  fail "run is DIAGNOSTIC_ONLY — not official satisfactory evidence (quick_rc=$QUICK_RC full_rc=$FULL_RC post_rc=$POST_RC seed_rc=${SEED_RC:-0})"
fi

pass "log $LOG — final_rc=$FINAL_RC termination_status=$TERM_STATUS; gates OK"
