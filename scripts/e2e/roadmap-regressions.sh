#!/usr/bin/env bash
# Native regression evidence for the Roadmap 0.9.2 F/EXQ rows.
#
# This is not an E2E replacement: it runs the cited unit/integration suites on
# the native host and binds every result to the exact source tree. Rows whose
# only support is an unversioned local E2E harness stay NOT EXECUTED.
#
# Usage:
#   scripts/e2e/roadmap-regressions.sh --expected-tree <sha256>
#                                      [--host LABEL] [--evidence-dir DIR]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CARGO_BIN="${CARGO_BIN:-cargo}"

fail() {
  printf 'ROADMAP_REGRESSIONS=FAIL code=%s reason=%s\n' "$1" "$2" >&2
  exit "$1"
}

tree_fingerprint() {
  local root="$1"
  {
    git -C "$root" rev-parse HEAD 2>/dev/null || return 1
    git -C "$root" status --porcelain=v1 --untracked-files=no 2>/dev/null || return 1
    (cd "$root" && git ls-files -z | LC_ALL=C sort -z | xargs -0 sha256sum) || return 1
  } | sha256sum | awk '{print $1}'
}

run_cargo() {
  local name="$1"
  shift
  local log="$OUT/$name.log"
  echo "=== $name"
  ( cd "$ROOT" && "$CARGO_BIN" "$@" ) >"$log" 2>&1
  local ec=$?
  local passed
  passed=$(grep -oE 'test result: ok\. [0-9]+ passed' "$log" | awk '{s+=$4} END{print s+0}')
  if [ "$ec" -ne 0 ]; then
    printf 'STEP=FAIL name=%s exit=%s log=%s\n' "$name" "$ec" "$log" | tee -a "$OUT/summary.txt"
    return 1
  fi
  if [ "${passed:-0}" -le 0 ]; then
    printf 'STEP=FAIL name=%s guard=zero_tests log=%s\n' "$name" "$log" | tee -a "$OUT/summary.txt"
    return 1
  fi
  printf 'STEP=PASS name=%s tests=%s\n' "$name" "$passed" | tee -a "$OUT/summary.txt"
  return 0
}

run_script() {
  local name="$1" marker="$2"
  shift 2
  local log="$OUT/$name.log"
  echo "=== $name"
  ( cd "$ROOT" && "$@" ) >"$log" 2>&1
  local ec=$?
  if [ "$ec" -ne 0 ] || ! grep -qE "$marker" "$log"; then
    printf 'STEP=FAIL name=%s exit=%s guard=marker log=%s\n' "$name" "$ec" "$log" | tee -a "$OUT/summary.txt"
    return 1
  fi
  printf 'STEP=PASS name=%s marker=ok\n' "$name" | tee -a "$OUT/summary.txt"
  return 0
}

expected=""
host="$(hostname 2>/dev/null || echo unknown)"
evidence=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --expected-tree) expected="${2:?}"; shift 2 ;;
    --host) host="${2:?}"; shift 2 ;;
    --evidence-dir) evidence="${2:?}"; shift 2 ;;
    *) fail 2 "unknown_argument:$1" ;;
  esac
done
[ -n "$expected" ] || fail 2 "expected_tree_required"

tree="$(tree_fingerprint "$ROOT")" || fail 2 "tree_unreadable"
[ "$tree" = "$expected" ] || fail 3 "tree_mismatch expected=$expected actual=$tree"
[ -z "$(git -C "$ROOT" status --porcelain=v1 --untracked-files=no)" ] \
  || fail 3 "tree_dirty"

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${evidence:-$ROOT/.exyonq-local/evidence/roadmap-regressions-$stamp}"
mkdir -p "$OUT" || fail 2 "evidence_dir_unwritable"
: >"$OUT/summary.txt"
{
  printf 'host_label=%s\n' "$host"
  printf 'hostname=%s\n' "$(hostname 2>/dev/null || echo unknown)"
  uname -a 2>&1 || true
  id 2>&1 || true
  nproc 2>&1 || true
  printf 'tree_fingerprint=%s\n' "$tree"
  printf 'head=%s\n' "$(git -C "$ROOT" rev-parse HEAD)"
} >"$OUT/host.txt" 2>&1

rc=0
run_cargo core_lib test --locked -p exyonq-core --lib || rc=1
run_cargo f01_body_deadline_spool test --locked -p exyonq-core --lib f01_ || rc=1
run_cargo f02_rule_fail_policy test --locked -p exyonq-waf --lib f02_ || rc=1
run_cargo f02_install_policy test --locked -p exyonq f02_ || rc=1
run_cargo f04_client_table_protection test --locked -p exyonq-core --lib f04_ || rc=1
run_cargo f05_websocket_limits test --locked -p exyonq-mod-proxy --lib f05_ || rc=1
run_cargo body_limit test --locked -p exyonq-core --test proxy_request_body_limit_test || rc=1
run_cargo reload_lifecycle test --locked -p exyonq-core --test reload_lifecycle || rc=1
run_cargo proxy_lib test --locked -p exyonq-mod-proxy --lib || rc=1
run_cargo fastcgi_lib test --locked -p exyonq-mod-fastcgi --lib || rc=1
run_cargo config_ir_lib test --locked -p exyonq-config-ir --lib || rc=1
run_cargo stream_config test --locked -p exyonq-config-ir --test stream_config || rc=1
run_cargo module_pipeline test --locked -p exyonq-module-pipeline --lib || rc=1
run_cargo waf_lib test --locked -p exyonq-waf --lib || rc=1
run_cargo control_api test --locked -p exyonq-control-api --lib || rc=1
run_cargo ops_runtime test --locked -p exyonq-ops-runtime --lib || rc=1
run_script f12_version_gate 'F12_.*PASS|RESULT=PASS|PASS' \
  bash scripts/gates/readme-release-version-gate.sh --tree "$ROOT" || rc=1

final="$(tree_fingerprint "$ROOT")" || fail 2 "tree_unreadable_final"
[ "$final" = "$tree" ] || fail 3 "tree_changed_during_run"
[ "$rc" -eq 0 ] || { printf 'ROADMAP_REGRESSIONS=FAIL\n' | tee -a "$OUT/summary.txt"; exit 6; }
{
  printf 'ROADMAP_REGRESSIONS=PASS\n'
  printf 'tree_fingerprint=%s\n' "$tree"
  printf 'head=%s\n' "$(git -C "$ROOT" rev-parse HEAD)"
} | tee -a "$OUT/summary.txt"
echo "EVIDENCE_DIR=$OUT"