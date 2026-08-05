#!/usr/bin/env bash
# Fail-closed check that release waivers declare expiry/revisit and required fields.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
REG="$ROOT/docs/governance/exceptions"

usage() {
  cat <<'EOF'
Usage:
  waiver-expiry-gate.sh --selftest
  waiver-expiry-gate.sh --check
EOF
}

require_fields() {
  local f="$1"
  local missing=0
  local key
  for key in EXCEPTION_ID VULNERABILITY_FIXED EXPIRY STATUS; do
    if ! grep -Eq "^EXCEPTION_ID = |^${key} = |^\| EXCEPTION_ID \|" "$f" \
      && ! grep -Eq "${key}" "$f"; then
      :
    fi
  done
  grep -Eq 'EXCEPTION_ID' "$f" || { echo "FAIL: $f missing EXCEPTION_ID"; missing=1; }
  grep -Eq 'VULNERABILITY_FIXED' "$f" || { echo "FAIL: $f missing VULNERABILITY_FIXED"; missing=1; }
  grep -Eqi 'EXPIRY|revisit|EXPIRY_REVIEW' "$f" || { echo "FAIL: $f missing expiry/revisit"; missing=1; }
  grep -Eq 'STATUS' "$f" || { echo "FAIL: $f missing STATUS"; missing=1; }
  return "$missing"
}

check_register() {
  local fail=0 f
  [[ -d "$REG" ]] || { echo "FAIL: missing $REG"; return 1; }
  shopt -s nullglob
  local files=("$REG"/*.md)
  shopt -u nullglob
  local count=0
  for f in "${files[@]}"; do
    [[ "$(basename "$f")" == "README.md" ]] && continue
    count=$((count + 1))
    require_fields "$f" || fail=1
    # Refuse wording that claims fixed when waived
    if grep -Eqi 'VULNERABILITY_FIXED = YES' "$f" && grep -Eqi 'WAIVED_FOR|ACTIVE_WAIVER' "$f"; then
      echo "FAIL: $f claims FIXED while waived"
      fail=1
    fi
  done
  if [[ "$count" -lt 1 ]]; then
    echo "FAIL: no exception records (expected at least RUSTSEC-2026-0222)"
    fail=1
  fi
  # Known live waiver must be present
  [[ -f "$REG/RUSTSEC-2026-0222.md" ]] || {
    echo "FAIL: CURRENT_RELEASE_WAIVERS_UNREGISTERED includes RUSTSEC-2026-0222"
    fail=1
  }
  if grep -Eq 'VULNERABILITY_FIXED = NO' "$REG/RUSTSEC-2026-0222.md" \
    && grep -Eq 'WAIVED_FOR_V043 = YES' "$REG/RUSTSEC-2026-0222.md" \
    && grep -Eq 'v0\.4\.4' "$REG/RUSTSEC-2026-0222.md"; then
    echo "RUSTSEC_2026_0222_EXCEPTION_STATUS=REGISTERED"
  else
    echo "FAIL: RUSTSEC-2026-0222 missing required waiver literals"
    fail=1
  fi
  if [[ "$fail" -ne 0 ]]; then
    echo "WAIVER_EXPIRY_ENFORCEMENT=FAIL"
    echo "CURRENT_RELEASE_WAIVERS_UNREGISTERED=nonzero"
    return 1
  fi
  echo "WAIVER_EXPIRY_ENFORCEMENT=PASS"
  echo "CURRENT_RELEASE_WAIVERS_UNREGISTERED=0"
  return 0
}

run_selftest() {
  local tmp
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-waiver-gate.XXXXXX")"
  cleanup() { rm -rf "${tmp:?}"; }
  trap cleanup EXIT
  mkdir -p "$tmp/docs/governance/exceptions"
  # MUST_FAIL: no expiry
  cat >"$tmp/docs/governance/exceptions/BAD.md" <<'EOF'
EXCEPTION_ID = BAD
VULNERABILITY_FIXED = NO
STATUS = ACTIVE_WAIVER
EOF
  if ROOT="$tmp" REG="$tmp/docs/governance/exceptions" check_register >/dev/null 2>&1; then
    echo "FAIL: expected bad register to fail"
    return 1
  fi
  echo "PASS_EXPECT_FAIL missing_expiry"
  # Real tree
  check_register
  trap - EXIT
  cleanup
  echo "WAIVER_EXPIRY_GATE_SELFTEST=PASS"
}

case "${1:-}" in
  --selftest) run_selftest ;;
  --check) check_register ;;
  *) usage >&2; exit 2 ;;
esac
