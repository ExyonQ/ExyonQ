#!/usr/bin/env bash
# Sealed launcher for scripts/e2e/proxy-datapath-stress.py (F-01/F-02/F-04/F-05).
#
# The tree and the binary must match the stamp written by native-matrix.sh:
# a stress result from a different source tree or from a stale binary is
# rejected instead of being reported as PASS.
#
# Usage:
#   scripts/e2e/proxy-datapath-stress.sh --expected-tree <sha256>
#       [--binary PATH] [--evidence-dir DIR] [--rss-limit-mib 16] [scenario]
#   scenario: f01|f02|f04|f05|all (default all)
#
# Exit codes: 0 pass; 2 usage; 3 tree mismatch; 4 stale/missing binary;
#             5 scenario failed or produced no PASS marker.
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
STAMP="$ROOT/target/.exyonq-native-matrix.stamp"
EXPECTED_TREE=""
OUT=""
RSS_LIMIT=16
SCENARIO="all"

usage() {
  sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

fail() {
  local code="$1"
  shift
  printf 'PROXY_DATAPATH_STRESS=FAIL code=%s reason=%s\n' "$code" "$*" >&2
  exit "$code"
}

tree_fingerprint() {
  {
    git -C "$ROOT" rev-parse HEAD 2>/dev/null || return 1
    git -C "$ROOT" status --porcelain=v1 --untracked-files=no 2>/dev/null || return 1
    (cd "$ROOT" && git ls-files -z | LC_ALL=C sort -z | xargs -0 sha256sum) || return 1
  } | sha256sum | awk '{print $1}'
}

sha256_of() {
  sha256sum "$1" 2>/dev/null | awk '{print $1}'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --expected-tree) EXPECTED_TREE="${2:-}"; shift 2 ;;
    --binary) BIN="${2:-}"; shift 2 ;;
    --evidence-dir) OUT="${2:-}"; shift 2 ;;
    --rss-limit-mib) RSS_LIMIT="${2:-16}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    f01|f02|f04|f05|all) SCENARIO="$1"; shift ;;
    *) usage >&2; exit 2 ;;
  esac
done

[ -n "$EXPECTED_TREE" ] || { usage >&2; fail 2 expected_tree_required; }
[ -x "$BIN" ] || fail 4 missing_binary
[ -f "$STAMP" ] || fail 4 missing_build_stamp

TREE="$(tree_fingerprint)" || fail 3 tree_unreadable
[ "$TREE" = "$EXPECTED_TREE" ] || fail 3 "tree_mismatch expected=$EXPECTED_TREE actual=$TREE"
STAMPED_TREE="$(awk '$1=="tree"{print $2}' "$STAMP")"
STAMPED_BIN="$(awk '$1=="bin"{print $2}' "$STAMP")"
CURRENT_BIN="$(sha256_of "$BIN")"
[ "$STAMPED_TREE" = "$TREE" ] || fail 4 "stale_stamp_tree expected=$STAMPED_TREE actual=$TREE"
[ "$STAMPED_BIN" = "$CURRENT_BIN" ] || fail 4 "stale_stamp_binary expected=$STAMPED_BIN actual=$CURRENT_BIN"

if [ -z "$OUT" ]; then
  OUT="$ROOT/.exyonq-local/evidence/proxy-datapath-stress/$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$OUT"
printf 'tree_fingerprint=%s\nhead=%s\nbinary sha256=%s\n' \
  "$TREE" "$(git -C "$ROOT" rev-parse HEAD)" "$CURRENT_BIN" >"$OUT/summary.txt"

scenarios=()
if [ "$SCENARIO" = all ]; then
  scenarios=(f01 f02 f04 f05)
else
  scenarios=("$SCENARIO")
fi

status=0
for scenario in "${scenarios[@]}"; do
  log="$OUT/$scenario.log"
  printf '=== %s\n' "$scenario" | tee -a "$OUT/summary.txt"
  # shellcheck disable=SC2086
  python3 "$SCRIPT_DIR/proxy-datapath-stress.py" "$scenario" \
    --binary "$BIN" --work "$OUT/$scenario-work" --rss-limit-mib "$RSS_LIMIT" \
    >"$log" 2>&1
  ec=$?
  tail -20 "$log" || true
  marker="$(printf '%s' "$scenario" | tr '[:lower:]' '[:upper:]')=PASS"
  if [ "$ec" -ne 0 ] || ! grep -qx "$marker" "$log"; then
    printf 'STEP=FAIL name=%s exit=%s marker=%s log=%s\n' \
      "$scenario" "$ec" "$marker" "$log" | tee -a "$OUT/summary.txt"
    status=1
  else
    printf 'STEP=PASS name=%s marker=ok\n' "$scenario" | tee -a "$OUT/summary.txt"
  fi
done

if [ "$status" -eq 0 ]; then
  printf 'PROXY_DATAPATH_STRESS=PASS tests=%s\n' "${#scenarios[@]}" | tee -a "$OUT/summary.txt"
  exit 0
fi
printf 'PROXY_DATAPATH_STRESS=FAIL\n' | tee -a "$OUT/summary.txt"
exit 5
