#!/usr/bin/env bash
# Fail-closed check that release waivers declare expiry/revisit and required fields.
# Supports ACTIVE_WAIVER records and REMEDIATED_D11 closed remediation records.
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
  grep -Eq 'EXCEPTION_ID' "$f" || { echo "FAIL: $f missing EXCEPTION_ID"; missing=1; }
  grep -Eq 'VULNERABILITY_FIXED' "$f" || { echo "FAIL: $f missing VULNERABILITY_FIXED"; missing=1; }
  grep -Eqi 'EXPIRY|revisit|EXPIRY_REVIEW' "$f" || { echo "FAIL: $f missing expiry/revisit"; missing=1; }
  grep -Eq 'STATUS' "$f" || { echo "FAIL: $f missing STATUS"; missing=1; }
  return "$missing"
}

check_rustsec_0222() {
  local f="$REG/RUSTSEC-2026-0222.md"
  [[ -f "$f" ]] || {
    echo "FAIL: CURRENT_RELEASE_WAIVERS_UNREGISTERED includes RUSTSEC-2026-0222"
    return 1
  }

  # Closed remediation path (D11+): fixed, waiver inactive, status remediates.
  if grep -Eq 'STATUS = REMEDIATED_D11' "$f" \
    && grep -Eq 'VULNERABILITY_FIXED = YES' "$f" \
    && grep -Eq 'WAIVER_ACTIVE = NO' "$f" \
    && grep -Eq '46\.0\.2' "$f"; then
    if grep -Eq 'STATUS = ACTIVE_WAIVER' "$f"; then
      echo "FAIL: $f REMEDIATED but still STATUS = ACTIVE_WAIVER"
      return 1
    fi
    if grep -Eq 'VULNERABILITY_FIXED = YES' "$f" && grep -Eq 'WAIVER_ACTIVE = YES' "$f"; then
      echo "FAIL: $f claims FIXED while WAIVER_ACTIVE = YES"
      return 1
    fi
    echo "RUSTSEC_2026_0222_EXCEPTION_STATUS=REMEDIATED_D11"
    return 0
  fi

  # Legacy active-waiver path (v0.4.3): unfixed + waived until v0.4.4.
  if grep -Eq 'VULNERABILITY_FIXED = NO' "$f" \
    && grep -Eq 'WAIVED_FOR_V043 = YES' "$f" \
    && grep -Eq 'v0\.4\.4' "$f"; then
    echo "RUSTSEC_2026_0222_EXCEPTION_STATUS=REGISTERED"
    return 0
  fi

  echo "FAIL: RUSTSEC-2026-0222 missing required ACTIVE_WAIVER or REMEDIATED_D11 literals"
  return 1
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
    # Refuse FIXED + active waiver flags on the same record.
    if grep -Eqi 'VULNERABILITY_FIXED = YES' "$f" \
      && grep -Eqi 'STATUS = ACTIVE_WAIVER|WAIVER_ACTIVE = YES' "$f"; then
      echo "FAIL: $f claims FIXED while waived/active"
      fail=1
    fi
  done
  if [[ "$count" -lt 1 ]]; then
    echo "FAIL: no exception records (expected at least RUSTSEC-2026-0222)"
    fail=1
  fi
  check_rustsec_0222 || fail=1
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

  # MUST_FAIL: FIXED + ACTIVE_WAIVER
  cat >"$tmp/docs/governance/exceptions/RUSTSEC-2026-0222.md" <<'EOF'
EXCEPTION_ID = RUSTSEC-2026-0222
STATUS = ACTIVE_WAIVER
VULNERABILITY_FIXED = YES
WAIVER_ACTIVE = YES
EXPIRY_REVIEW = v0.4.4
EOF
  if ROOT="$tmp" REG="$tmp/docs/governance/exceptions" check_register >/dev/null 2>&1; then
    echo "FAIL: expected FIXED+ACTIVE to fail"
    return 1
  fi
  echo "PASS_EXPECT_FAIL fixed_while_active"

  # MUST_PASS: REMEDIATED_D11 fixture (drop prior BAD.md so only remediates record remains)
  rm -f "$tmp/docs/governance/exceptions/BAD.md"
  cat >"$tmp/docs/governance/exceptions/RUSTSEC-2026-0222.md" <<'EOF'
EXCEPTION_ID = RUSTSEC-2026-0222
STATUS = REMEDIATED_D11
VULNERABILITY_FIXED = YES
WAIVER_ACTIVE = NO
EXPIRY_REVIEW = CLOSED_BY_D11
WASMTIME_VERSION = 46.0.2
EOF
  if ! ROOT="$tmp" REG="$tmp/docs/governance/exceptions" check_register >/dev/null 2>&1; then
    echo "FAIL: expected REMEDIATED_D11 fixture to pass"
    return 1
  fi
  echo "PASS_EXPECT_PASS remediated_d11"

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
