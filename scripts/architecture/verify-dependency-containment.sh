#!/usr/bin/env bash
# DB2C1–DB2C3 — Dependency containment gates (manifest, provider surface, DBEX/hot-path/dups + D1).
#
# Usage:
#   bash scripts/architecture/verify-dependency-containment.sh --check
#   bash scripts/architecture/verify-dependency-containment.sh --json
#   bash scripts/architecture/verify-dependency-containment.sh --explain
#   bash scripts/architecture/verify-dependency-containment.sh --self-test-negative
#
# Exit codes (aligned with scripts/architecture/*):
#   0 = PASS
#   1 = POLICY_VIOLATION
#   2 = CONFIGURATION_OR_TOOL_FAILURE
set -euo pipefail

_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_ROOT="$(cd "$_DIR/../.." && pwd)"
_CHECK_DIR="$_DIR/dependency_containment"

MODE="check"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) MODE="check"; shift ;;
    --json) MODE="json"; shift ;;
    --explain) MODE="explain"; shift ;;
    --self-test-negative) MODE="self-test-negative"; shift ;;
    -h|--help)
      sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "verify-dependency-containment: unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

cd "$_ROOT"

if ! command -v python3 >/dev/null 2>&1; then
  echo "verify-dependency-containment: python3 required on PATH" >&2
  exit 2
fi

if [[ "$MODE" == "explain" ]]; then
  PYTHONPATH="$_CHECK_DIR" python3 -c 'from common import explain_db2c1; print(explain_db2c1())'
  exit 0
fi

if [[ "$MODE" == "self-test-negative" ]]; then
  export PYTHONPATH="$_CHECK_DIR${PYTHONPATH:+:$PYTHONPATH}"
  python3 "$_CHECK_DIR/run_negative_validation.py"
  exit $?
fi

export PYTHONPATH="$_CHECK_DIR${PYTHONPATH:+:$PYTHONPATH}"

ERRORS=0
WARNINGS=0
JSON_MODE=0
[[ "$MODE" == "json" ]] && JSON_MODE=1

run_py() {
  local script="$1"
  local out rc
  set +e
  if [[ "$JSON_MODE" -eq 1 ]]; then
    out="$(python3 "$_CHECK_DIR/$script" --root "$_ROOT" --json 2>&1)"
  else
    out="$(python3 "$_CHECK_DIR/$script" --root "$_ROOT" 2>&1)"
  fi
  rc=$?
  set -e
  printf '%s\n' "$out"
  if [[ "$rc" -eq 2 ]]; then
    echo "verify-dependency-containment: tool/config failure in $script" >&2
    exit 2
  fi
  if [[ "$rc" -ne 0 ]]; then
    ERRORS=1
  fi
  # count WARN findings (human [WARN] or JSON "severity": "WARN")
  local w
  if [[ "$JSON_MODE" -eq 1 ]]; then
    w="$(printf '%s\n' "$out" | grep -c '"severity": "WARN"' || true)"
  else
    w="$(printf '%s\n' "$out" | grep -c '\[WARN\]' || true)"
  fi
  WARNINGS=$((WARNINGS + w))
}

echo "verify-dependency-containment: root=$_ROOT mode=$MODE"

run_py check_manifest_registry.py
run_py check_manifest_zones.py
run_py check_lock_and_manifest_policy.py
run_py check_dependency_admissions.py

# DB2C2 — provider surface gates
run_py check_source_import_zones.py
run_py check_public_provider_surface.py
run_py check_provider_errors.py
run_py check_critical_provider_owners.py

# DB2C3 — exceptions / hot-path / duplicates
run_py check_dependency_exceptions.py
run_py check_hot_path_budget.py
run_py check_duplicate_dependencies.py

# GATE-DEP-010 — reuse existing D1 script (never duplicate)
echo "verify-dependency-containment: invoking verify-ps3a-platform-linux-d1.sh"
set +e
bash "$_DIR/verify-ps3a-platform-linux-d1.sh"
d1_rc=$?
set -e
if [[ "$d1_rc" -eq 2 ]]; then
  echo "verify-dependency-containment: D1 tool/config failure" >&2
  exit 2
fi
if [[ "$d1_rc" -ne 0 ]]; then
  echo "verify-dependency-containment: D1 POLICY_VIOLATION" >&2
  ERRORS=1
fi

if [[ "$ERRORS" -ne 0 ]]; then
  echo "verify-dependency-containment: FAIL errors=1 warnings=$WARNINGS"
  exit 1
fi
echo "verify-dependency-containment: PASS warnings=$WARNINGS"
exit 0
