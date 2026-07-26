#!/usr/bin/env bash
# P1.5-WS2 — summarize evidence directory metas.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EV="${1:-}"
if [[ -z "$EV" ]]; then
  echo "usage: $0 <evidence-dir>" >&2
  exit 2
fi
[[ -d "$EV" ]] || { echo "FATAL: missing $EV" >&2; exit 2; }

echo "=== P15-WS2 summary $EV ==="
PASS=0; FAIL=0; TOTAL_CRASH_FILES=0
for m in "$EV"/*.meta.env; do
  [[ -f "$m" ]] || continue
  t=""; mode=""; exitc=""; crashes=""
  while IFS= read -r line; do
    case "$line" in
      TARGET=*) t="${line#TARGET=}" ;;
      MODE=*) mode="${line#MODE=}" ;;
      EXIT=*) exitc="${line#EXIT=}" ;;
      CRASHES=*) crashes="${line#CRASHES=}" ;;
    esac
  done <"$m"
  echo "TARGET=$t MODE=$mode EXIT=$exitc CRASHES=$crashes"
  if [[ "${exitc:-1}" -eq 0 ]]; then PASS=$((PASS+1)); else FAIL=$((FAIL+1)); fi
  TOTAL_CRASH_FILES=$((TOTAL_CRASH_FILES + ${crashes:-0}))
done
echo "PASS_TARGETS=$PASS FAIL_TARGETS=$FAIL TOTAL_CRASH_FILES=$TOTAL_CRASH_FILES"
if [[ "$FAIL" -ne 0 ]]; then
  echo "P15_WS2_SUMMARY=FAIL"
  exit 1
fi
echo "P15_WS2_SUMMARY=PASS"
