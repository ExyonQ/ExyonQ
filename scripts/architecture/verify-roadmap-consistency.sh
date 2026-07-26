#!/usr/bin/env bash
# verify-roadmap-consistency.sh — Phase 1 roadmap SSOT consistency gate (docs-only).
#
# Checks canonical roadmap documents under docs/roadmap/ for single-SSOT,
# block sequencing, and hard strategic invariants from P1.0.
#
# Usage: bash scripts/architecture/verify-roadmap-consistency.sh
# Exit: 0 PASS, 1 FAIL
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ROADMAP_DIR="$ROOT/docs/roadmap"
SSOT="$ROADMAP_DIR/CANONICAL_PROJECT_STATE.md"
ROADMAP="$ROADMAP_DIR/ROADMAP.md"
ARCHIVE_MR="$ROADMAP_DIR/archive/modern-rivals-diagnostics.md"
ACTIVE_DIR="$ROADMAP_DIR/active"
COMPLETED_DIR="$ROADMAP_DIR/completed"

errors=0
warn() { echo "verify-roadmap-consistency: WARNING: $*"; }
fail() { echo "verify-roadmap-consistency: FAIL: $*"; errors=$((errors + 1)); }
pass() { echo "verify-roadmap-consistency: PASS: $*"; }

require_file() {
  local f="$1"
  if [[ ! -f "$f" ]]; then
    fail "missing required file: ${f#"$ROOT"/}"
    return 1
  fi
  pass "exists ${f#"$ROOT"/}"
}

require_contains() {
  local f="$1" pat="$2" msg="$3"
  if ! rg -q -- "$pat" "$f"; then
    fail "$msg (pattern not found in ${f#"$ROOT"/}: $pat)"
  else
    pass "$msg"
  fi
}

forbid_contains() {
  local f="$1" pat="$2" msg="$3"
  if rg -q -- "$pat" "$f"; then
    fail "$msg (forbidden pattern in ${f#"$ROOT"/}: $pat)"
  else
    pass "$msg"
  fi
}

echo "verify-roadmap-consistency: scanning root=$ROOT"

require_file "$SSOT"
require_file "$ROADMAP"
require_file "$ARCHIVE_MR"
require_file "$ROADMAP_DIR/PHASE1_SERVER_PRODUCT_CLOSURE_AUDIT.md"

# Single SSOT markers
require_contains "$SSOT" 'SSOT_AUTHORITY[[:space:]]*=[[:space:]]*YES' "SSOT declares authority"
require_contains "$ROADMAP" 'CANONICAL_PROJECT_STATE\.md' "ROADMAP links to SSOT"
require_contains "$SSOT" 'EXYONQ_PLATFORM_LINUX_EXISTS[[:space:]]*=[[:space:]]*YES' "platform-linux exists"
require_contains "$SSOT" 'PLATFORM_SPLIT_PHYSICAL[[:space:]]*=[[:space:]]*CLOSED_TECHNICALLY' "platform split technically closed"
require_contains "$SSOT" 'PLATFORM_DIRECTION_D1[[:space:]]*=[[:space:]]*PRESERVED' "D1 preserved"
require_contains "$SSOT" 'DEPENDENCY_BOUNDARY_PROGRAM[[:space:]]*=[[:space:]]*CLOSED' "dependency containment program closed"
require_contains "$SSOT" 'CACHE_LINE[[:space:]]*=[[:space:]]*CLOSED' "cache line closed"
require_contains "$SSOT" 'HTTP3_PRODUCTION_CLAIM_ALLOWED_BEFORE_P1\.3b[[:space:]]*=[[:space:]]*NO' "HTTP/3 not production-ready yet"
require_contains "$SSOT" 'OFFICIAL_BENCHMARK_BLOCK[[:space:]]*=[[:space:]]*P1\.7' "official bench only P1.7"
require_contains "$ARCHIVE_MR" 'MR7_MR8_CLASSIFICATION[[:space:]]*=[[:space:]]*DIAGNOSTIC_ARCHIVE' "MR7/MR8 diagnostic"
require_contains "$ARCHIVE_MR" 'OFFICIAL_CLAIM_ALLOWED[[:space:]]*=[[:space:]]*NO' "MR official claim forbidden"
# Current-state assignments only: match start-of-line / fence assignments, not historical prose tables.
if rg -n '^[[:space:]]*PS2[[:space:]]*=[[:space:]]*OPEN\b' "$SSOT" \
  || rg -n '^PS2[[:space:]]*=[[:space:]]*OPEN\b' "$SSOT"; then
  fail "SSOT must not claim PS2 OPEN as a current assignment"
else
  pass "SSOT must not claim PS2 OPEN as current"
fi
if rg -n 'EXYONQ_PLATFORM_LINUX_EXISTS[[:space:]]*=[[:space:]]*NO\b' "$SSOT" \
  || rg -n '^PLATFORM_LINUX_CRATE_EXISTS[[:space:]]*=[[:space:]]*NO\b' "$SSOT"; then
  fail "SSOT must not deny platform-linux as a current assignment"
else
  pass "SSOT must not deny platform-linux"
fi
forbid_contains "$ROADMAP" 'MR7.*OFFICIAL|OFFICIAL.*MR7' "ROADMAP must not call MR7 official"
forbid_contains "$ARCHIVE_MR" 'REUSE_AS_OFFICIAL[[:space:]]*=[[:space:]]*YES' "archive must not allow official reuse"

if rg -n 'PLATFORM_SPLIT_PHYSICAL[[:space:]]*=[[:space:]]*NOT_STARTED\b' "$SSOT"; then
  fail "SSOT still asserts PLATFORM_SPLIT_PHYSICAL = NOT_STARTED as current"
else
  pass "no current PLATFORM_SPLIT_PHYSICAL = NOT_STARTED"
fi

# Platform crate on disk
if [[ -d "$ROOT/crates/exyonq-platform-linux" ]]; then
  pass "crates/exyonq-platform-linux present"
else
  fail "crates/exyonq-platform-linux missing"
fi

# Active block policy: at most one active phase-1.* file
active_p10=0
active_p11=0
active_p12=0
active_p13a=0
active_p13b=0
active_p14=0
active_p15=0
active_p16=0
active_p17=0
if [[ -d "$ACTIVE_DIR" ]]; then
  while IFS= read -r -d '' f; do
    base="$(basename "$f")"
    case "$base" in
      phase-1.0-*) active_p10=$((active_p10 + 1)) ;;
      phase-1.1-*) active_p11=$((active_p11 + 1)) ;;
      phase-1.2-*) active_p12=$((active_p12 + 1)) ;;
      phase-1.3a-*) active_p13a=$((active_p13a + 1)) ;;
      phase-1.3b-*) active_p13b=$((active_p13b + 1)) ;;
      phase-1.4-*) active_p14=$((active_p14 + 1)) ;;
      phase-1.5-*) active_p15=$((active_p15 + 1)) ;;
      phase-1.6-*) active_p16=$((active_p16 + 1)) ;;
      phase-1.7-*) active_p17=$((active_p17 + 1)) ;;
      *)
        fail "unexpected file in active/: $base"
        ;;
    esac
  done < <(find "$ACTIVE_DIR" -maxdepth 1 -type f -name 'phase-1*.md' -print0 2>/dev/null || true)
fi

if [[ $((active_p10 + active_p11 + active_p12 + active_p13a + active_p13b + active_p14 + active_p15 + active_p16 + active_p17)) -gt 1 ]]; then
  fail "ONE_ACTIVE_BLOCK violated: multiple phase-1.* files in active/"
fi

completed_p10=0
completed_p11=0
completed_p12=0
completed_p13a=0
completed_p13b=0
completed_p14=0
completed_p15=0
completed_p16=0
if [[ -f "$COMPLETED_DIR/phase-1.0-canonical-state-refresh.md" ]]; then
  completed_p10=1
fi
if [[ -f "$COMPLETED_DIR/phase-1.1-correctness-and-stability.md" ]]; then
  completed_p11=1
fi
if [[ -f "$COMPLETED_DIR/phase-1.2-fastcgi-php-fpm-production-grade.md" ]]; then
  completed_p12=1
fi
if [[ -f "$COMPLETED_DIR/phase-1.3a-proxy-tls-h2-product-completion.md" ]]; then
  completed_p13a=1
fi
if [[ -f "$COMPLETED_DIR/phase-1.3b-http3-production-readiness.md" ]]; then
  completed_p13b=1
fi
if [[ -f "$COMPLETED_DIR/phase-1.4-configuration-product-completion.md" ]]; then
  completed_p14=1
fi
if [[ -f "$COMPLETED_DIR/phase-1.5-security-soak-ops-release-hardening.md" ]]; then
  completed_p15=1
fi
if [[ -f "$COMPLETED_DIR/phase-1.6-release-candidate-freeze.md" ]]; then
  completed_p16=1
fi

if [[ "$active_p10" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.0' "ROADMAP ACTIVE_BLOCK=P1.0 while active file exists"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.0' "SSOT ACTIVE_BLOCK=P1.0"
  if [[ "$completed_p10" -eq 1 ]]; then
    fail "P1.0 present in both active/ and completed/ (divergent dual copy)"
  fi
elif [[ "$active_p11" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.1' "ROADMAP ACTIVE_BLOCK=P1.1"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.1' "SSOT ACTIVE_BLOCK=P1.1"
  require_contains "$SSOT" 'P1_1_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "SSOT P1.1 ACTIVE"
  require_contains "$ACTIVE_DIR/phase-1.1-correctness-and-stability.md" 'P1_1_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "active P1.1 ACTIVE"
  if [[ "$completed_p10" -ne 1 ]]; then
    fail "P1.1 active requires completed P1.0"
  fi
  if [[ "$completed_p11" -eq 1 ]]; then
    fail "P1.1 present in both active/ and completed/"
  fi
  pass "P1.1 is sole ACTIVE block"
elif [[ "$active_p12" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.2' "ROADMAP ACTIVE_BLOCK=P1.2"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.2' "SSOT ACTIVE_BLOCK=P1.2"
  require_contains "$SSOT" 'P1_2_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "SSOT P1.2 ACTIVE"
  require_contains "$ACTIVE_DIR/phase-1.2-fastcgi-php-fpm-production-grade.md" 'P1_2_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "active P1.2 ACTIVE"
  require_contains "$SSOT" 'FASTCGI_PRODUCT_STATUS[[:space:]]*=[[:space:]]*FUNCTIONAL_BUT_INCOMPLETE' "FastCGI not product-ready while P1.2 active"
  if [[ "$completed_p11" -ne 1 ]]; then
    fail "P1.2 active requires completed P1.1"
  fi
  if [[ "$completed_p12" -eq 1 ]]; then
    fail "P1.2 present in both active/ and completed/"
  fi
  pass "P1.2 is sole ACTIVE block"
elif [[ "$active_p13a" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.3a' "ROADMAP ACTIVE_BLOCK=P1.3a"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.3a' "SSOT ACTIVE_BLOCK=P1.3a"
  require_contains "$SSOT" 'P1_3A_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "SSOT P1.3a ACTIVE"
  require_contains "$ROADMAP" 'P1_3A_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "ROADMAP P1.3a ACTIVE"
  require_contains "$ACTIVE_DIR/phase-1.3a-proxy-tls-h2-product-completion.md" 'P1_3A_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "active P1.3a ACTIVE"
  require_contains "$SSOT" 'P1_2_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.2 CLOSED while P1.3a active"
  require_contains "$SSOT" 'FASTCGI_PRODUCT_STATUS[[:space:]]*=[[:space:]]*PRODUCT_CLOSED' "FastCGI remains PRODUCT_CLOSED during P1.3a"
  require_contains "$SSOT" 'PROXY_PRODUCT_STATUS[[:space:]]*=[[:space:]]*FUNCTIONAL_BUT_INCOMPLETE' "Proxy not product-ready while P1.3a active"
  require_contains "$SSOT" 'AUTO_OPEN_P1_3B[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_3B=NO"
  if [[ "$completed_p12" -ne 1 ]]; then
    fail "P1.3a active requires completed P1.2"
  fi
  if [[ "$completed_p13a" -eq 1 ]]; then
    fail "P1.3a present in both active/ and completed/"
  fi
  pass "P1.3a is sole ACTIVE block"
elif [[ "$active_p13b" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.3b' "ROADMAP ACTIVE_BLOCK=P1.3b"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.3b' "SSOT ACTIVE_BLOCK=P1.3b"
  require_contains "$SSOT" 'P1_3B_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "SSOT P1.3b ACTIVE"
  require_contains "$ROADMAP" 'P1_3B_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "ROADMAP P1.3b ACTIVE"
  require_contains "$ACTIVE_DIR/phase-1.3b-http3-production-readiness.md" 'P1_3B_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "active P1.3b ACTIVE"
  require_contains "$SSOT" 'P1_3A_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.3a CLOSED while P1.3b active"
  require_contains "$SSOT" 'HTTP3_PRODUCT_STATUS[[:space:]]*=[[:space:]]*EXPERIMENTAL' "HTTP/3 not product-ready while P1.3b active"
  require_contains "$SSOT" 'AUTO_OPEN_P1_4[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_4=NO"
  if [[ "$completed_p13a" -ne 1 ]]; then
    fail "P1.3b active requires completed P1.3a"
  fi
  if [[ "$completed_p13b" -eq 1 ]]; then
    fail "P1.3b present in both active/ and completed/"
  fi
  pass "P1.3b is sole ACTIVE block"
elif [[ "$active_p14" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.4' "ROADMAP ACTIVE_BLOCK=P1.4"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.4' "SSOT ACTIVE_BLOCK=P1.4"
  require_contains "$SSOT" 'P1_4_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "SSOT P1.4 ACTIVE"
  require_contains "$ROADMAP" 'P1_4_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "ROADMAP P1.4 ACTIVE"
  require_contains "$ACTIVE_DIR/phase-1.4-configuration-product-completion.md" 'P1_4_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "active P1.4 ACTIVE"
  require_contains "$SSOT" 'P1_3B_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.3b CLOSED while P1.4 active"
  require_contains "$SSOT" 'HTTP3_PRODUCT_STATUS[[:space:]]*=[[:space:]]*PRODUCT_CLOSED' "HTTP/3 remains product-closed during P1.4"
  require_contains "$SSOT" 'AUTO_OPEN_P1_5[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_5=NO"
  require_contains "$SSOT" 'PRODUCT_CODE_P1_4[[:space:]]*=[[:space:]]*FORBIDDEN_UNTIL_SEPARATE_ADMIT' "P1.4 product code gated"
  if [[ "$completed_p13b" -ne 1 ]]; then
    fail "P1.4 active requires completed P1.3b"
  fi
  if [[ -f "$COMPLETED_DIR/phase-1.4-configuration-product-completion.md" ]]; then
    fail "P1.4 present in both active/ and completed/"
  fi
  pass "P1.4 is sole ACTIVE block"
elif [[ "$active_p15" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.5' "ROADMAP ACTIVE_BLOCK=P1.5"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.5' "SSOT ACTIVE_BLOCK=P1.5"
  require_contains "$SSOT" 'P1_5_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "SSOT P1.5 ACTIVE"
  require_contains "$ROADMAP" 'P1_5_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "ROADMAP P1.5 ACTIVE"
  require_contains "$ACTIVE_DIR/phase-1.5-security-soak-ops-release-hardening.md" 'P1_5_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "active P1.5 ACTIVE"
  require_contains "$SSOT" 'P1_4_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.4 CLOSED while P1.5 active"
  require_contains "$SSOT" 'AUTO_OPEN_P1_6[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_6=NO"
  require_contains "$SSOT" 'P1_6_OPENED[[:space:]]*=[[:space:]]*NO' "P1_6_OPENED=NO"
  require_contains "$SSOT" 'PRODUCT_CODE_P1_5[[:space:]]*=[[:space:]]*FORBIDDEN_UNTIL_SEPARATE_ADMIT' "P1.5 product code gated"
  if [[ "$completed_p14" -ne 1 ]]; then
    fail "P1.5 active requires completed P1.4"
  fi
  if [[ -f "$COMPLETED_DIR/phase-1.5-security-soak-ops-release-hardening.md" ]]; then
    fail "P1.5 present in both active/ and completed/"
  fi
  pass "P1.5 is sole ACTIVE block"
elif [[ "$active_p16" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.6' "ROADMAP ACTIVE_BLOCK=P1.6"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.6' "SSOT ACTIVE_BLOCK=P1.6"
  require_contains "$SSOT" 'P1_6_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "SSOT P1.6 ACTIVE"
  require_contains "$ROADMAP" 'P1_6_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "ROADMAP P1.6 ACTIVE"
  require_contains "$ACTIVE_DIR/phase-1.6-release-candidate-freeze.md" 'P1_6_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "active P1.6 ACTIVE"
  require_contains "$SSOT" 'P1_5_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.5 CLOSED while P1.6 active"
  require_contains "$ROADMAP" 'P1_5_STATUS[[:space:]]*=[[:space:]]*CLOSED' "ROADMAP P1.5 CLOSED while P1.6 active"
  require_contains "$COMPLETED_DIR/phase-1.5-security-soak-ops-release-hardening.md" 'P1_5_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.5 CLOSED"
  require_contains "$SSOT" 'RC_STATUS[[:space:]]*=[[:space:]]*DECLARED_INTERNAL_NOT_RELEASED' "SSOT RC_STATUS=DECLARED_INTERNAL_NOT_RELEASED"
  require_contains "$ROADMAP" 'RC_STATUS[[:space:]]*=[[:space:]]*DECLARED_INTERNAL_NOT_RELEASED' "ROADMAP RC_STATUS=DECLARED_INTERNAL_NOT_RELEASED"
  require_contains "$ACTIVE_DIR/phase-1.6-release-candidate-freeze.md" 'RC_STATUS[[:space:]]*=[[:space:]]*DECLARED_INTERNAL_NOT_RELEASED' "active P1.6 RC_STATUS=DECLARED_INTERNAL_NOT_RELEASED"
  require_contains "$SSOT" 'P1_6_WS6[[:space:]]*=[[:space:]]*PASS' "SSOT P1_6_WS6=PASS"
  require_contains "$SSOT" 'READY_FOR_P1_6_WS7_ADMIT[[:space:]]*=[[:space:]]*YES' "SSOT READY_FOR_P1_6_WS7_ADMIT=YES"
  require_contains "$SSOT" 'AUTO_OPEN_WS7[[:space:]]*=[[:space:]]*NO' "SSOT AUTO_OPEN_WS7=NO"
  require_contains "$SSOT" 'AUTO_OPEN_P1_7[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_7=NO"
  require_contains "$SSOT" 'P1_7_OPENED[[:space:]]*=[[:space:]]*NO' "P1_7_OPENED=NO"
  require_contains "$SSOT" 'PRODUCT_CODE_P1_6[[:space:]]*=[[:space:]]*FORBIDDEN_UNTIL_SEPARATE_ADMIT' "P1.6 product code gated"
  require_contains "$SSOT" 'P1_5_RELEASE_ARTIFACTS_ARE_RC[[:space:]]*=[[:space:]]*NO' "P1.5 artifacts are not RC"
  require_contains "$SSOT" 'PUBLIC_RELEASE[[:space:]]*=[[:space:]]*NO' "PUBLIC_RELEASE=NO after internal RC declare"
  require_contains "$SSOT" 'OFFICIAL_BENCHMARK_BLOCK[[:space:]]*=[[:space:]]*P1\.7' "official bench remains P1.7"
  if [[ "$completed_p15" -ne 1 ]]; then
    fail "P1.6 active requires completed P1.5"
  fi
  if [[ -f "$COMPLETED_DIR/phase-1.6-release-candidate-freeze.md" ]]; then
    fail "P1.6 present in both active/ and completed/"
  fi
  pass "P1.6 is sole ACTIVE block"
elif [[ "$active_p17" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.7' "ROADMAP ACTIVE_BLOCK=P1.7"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*P1\.7' "SSOT ACTIVE_BLOCK=P1.7"
  require_contains "$SSOT" 'P1_7_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "SSOT P1.7 ACTIVE"
  require_contains "$ROADMAP" 'P1_7_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "ROADMAP P1.7 ACTIVE"
  require_contains "$ACTIVE_DIR/phase-1.7-official-dual-platform-benchmark.md" 'P1_7_STATUS[[:space:]]*=[[:space:]]*ACTIVE' "active P1.7 ACTIVE"
  require_contains "$SSOT" 'P1_6_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.6 CLOSED while P1.7 active"
  require_contains "$ROADMAP" 'P1_6_STATUS[[:space:]]*=[[:space:]]*CLOSED' "ROADMAP P1.6 CLOSED while P1.7 active"
  require_contains "$COMPLETED_DIR/phase-1.6-release-candidate-freeze.md" 'P1_6_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.6 CLOSED"
  require_contains "$SSOT" 'RC_STATUS[[:space:]]*=[[:space:]]*DECLARED_INTERNAL_NOT_RELEASED' "SSOT RC_STATUS=DECLARED_INTERNAL_NOT_RELEASED"
  require_contains "$ROADMAP" 'RC_STATUS[[:space:]]*=[[:space:]]*DECLARED_INTERNAL_NOT_RELEASED' "ROADMAP RC_STATUS=DECLARED_INTERNAL_NOT_RELEASED"
  require_contains "$SSOT" 'P1_7_OPENED[[:space:]]*=[[:space:]]*YES' "P1_7_OPENED=YES"
  require_contains "$SSOT" 'READY_TO_OPEN_P1_7[[:space:]]*=[[:space:]]*CONSUMED' "READY_TO_OPEN_P1_7=CONSUMED"
  require_contains "$SSOT" 'AUTO_OPEN_P1_7[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_7=NO"
  require_contains "$SSOT" 'AUTO_OPEN_WS1[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_WS1=NO"
  if rg -q 'P1_7_WS2[[:space:]]*=[[:space:]]*PASS' "$SSOT"; then
    require_contains "$SSOT" 'STOP_BEFORE_WS1[[:space:]]*=[[:space:]]*CONSUMED' "STOP_BEFORE_WS1=CONSUMED after WS1 PASS"
    require_contains "$SSOT" 'STOP_BEFORE_WS2[[:space:]]*=[[:space:]]*CONSUMED' "STOP_BEFORE_WS2=CONSUMED after WS2 PASS"
    require_contains "$ACTIVE_DIR/phase-1.7-official-dual-platform-benchmark.md" 'P1_7_WS2[[:space:]]*=[[:space:]]*PASS' "active P1.7 WS2 PASS"
    require_contains "$SSOT" 'STOP_BEFORE_WS3[[:space:]]*=[[:space:]]*YES' "STOP_BEFORE_WS3=YES"
    require_contains "$SSOT" 'READY_FOR_P1_7_WS3_ADMIT[[:space:]]*=[[:space:]]*YES' "READY_FOR_P1_7_WS3_ADMIT=YES"
    require_contains "$SSOT" 'AUTO_OPEN_WS3[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_WS3=NO"
    require_contains "$SSOT" 'P1_7_EXECUTION_LOCK_STATUS[[:space:]]*=[[:space:]]*RESOLVING' "lock RESOLVING after WS2"
    require_file "$ROOT/docs/benchmarks/p1.7-rival-identity-register.md"
    require_file "$ROOT/docs/benchmarks/p1.7-rival-supply-chain-review.md"
    require_file "$ROOT/benchmarks/p1.7/identities/rivals.json"
  elif rg -q 'P1_7_WS1[[:space:]]*=[[:space:]]*PASS' "$SSOT"; then
    require_contains "$SSOT" 'STOP_BEFORE_WS1[[:space:]]*=[[:space:]]*CONSUMED' "STOP_BEFORE_WS1=CONSUMED after WS1 PASS"
    require_contains "$ACTIVE_DIR/phase-1.7-official-dual-platform-benchmark.md" 'P1_7_WS1[[:space:]]*=[[:space:]]*PASS' "active P1.7 WS1 PASS"
    require_contains "$SSOT" 'STOP_BEFORE_WS2[[:space:]]*=[[:space:]]*YES' "STOP_BEFORE_WS2=YES"
    require_contains "$SSOT" 'READY_FOR_P1_7_WS2_ADMIT[[:space:]]*=[[:space:]]*YES' "READY_FOR_P1_7_WS2_ADMIT=YES"
    require_contains "$SSOT" 'AUTO_OPEN_WS2[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_WS2=NO"
    require_contains "$SSOT" 'P1_7_EXECUTION_LOCK_STATUS[[:space:]]*=[[:space:]]*DRAFT_TEMPLATE_ONLY' "DRAFT template only"
  else
    require_contains "$SSOT" 'STOP_BEFORE_WS1[[:space:]]*=[[:space:]]*YES' "STOP_BEFORE_WS1=YES"
    require_contains "$ACTIVE_DIR/phase-1.7-official-dual-platform-benchmark.md" 'STOP_BEFORE_WS1[[:space:]]*=[[:space:]]*YES' "active P1.7 STOP_BEFORE_WS1=YES"
  fi
  require_contains "$SSOT" 'BENCHMARK_EXECUTION[[:space:]]*=[[:space:]]*FORBIDDEN_UNTIL_SEPARATE_ADMIT' "BENCHMARK_EXECUTION gated"
  require_contains "$SSOT" 'PRODUCT_CODE_CHANGE[[:space:]]*=[[:space:]]*FORBIDDEN_UNTIL_SEPARATE_ADMIT' "PRODUCT_CODE_CHANGE gated"
  require_contains "$SSOT" 'PERFORMANCE_TUNING[[:space:]]*=[[:space:]]*FORBIDDEN_UNTIL_SEPARATE_ADMIT' "PERFORMANCE_TUNING gated"
  require_contains "$SSOT" 'PUBLICATION_STATUS[[:space:]]*=[[:space:]]*FORBIDDEN' "PUBLICATION_STATUS=FORBIDDEN"
  require_contains "$SSOT" 'OFFICIAL_CLAIM_ALLOWED[[:space:]]*=[[:space:]]*NO' "OFFICIAL_CLAIM_ALLOWED=NO"
  require_contains "$SSOT" 'OFFICIAL_BENCHMARK_BLOCK[[:space:]]*=[[:space:]]*P1\.7' "official bench remains P1.7"
  require_contains "$SSOT" 'PHASE_1_PRODUCT_CLOSED[[:space:]]*=[[:space:]]*NO' "Phase 1 not complete while P1.7 pending"
  require_contains "$SSOT" 'EXYONQ_BENCHMARK_IDENTITY_RECOMMENDATION[[:space:]]*=[[:space:]]*RC_ARTIFACTS_0_3_3_RC_1' "RC artifact identity recommendation"
  require_contains "$SSOT" 'RC_ARTIFACT_SOURCE_HEAD[[:space:]]*=[[:space:]]*4446f91c7d8669b828b978e463ab3338391debbb' "RC artifact source head pinned"
  require_contains "$SSOT" 'BENCHMARK_PROGRAM_HEAD[[:space:]]*=[[:space:]]*68aaef5706c0a47c051f3a148c701edf748f3f7e' "benchmark program head pinned"
  if rg -q 'RC_ARTIFACT_SOURCE_HEAD[[:space:]]*=[[:space:]]*68aaef5706c0a47c051f3a148c701edf748f3f7e' "$SSOT"; then
    fail "RC_ARTIFACT_SOURCE_HEAD must not equal BENCHMARK_PROGRAM_HEAD"
  else
    pass "RC artifact head distinct from program head"
  fi
  require_file "$ROOT/docs/benchmarks/p1.7-official-benchmark-current-state-audit.md"
  require_file "$ROOT/docs/benchmarks/p1.7-official-benchmark-contract.md"
  if [[ "$completed_p16" -ne 1 ]]; then
    fail "P1.7 active requires completed P1.6"
  fi
  if [[ -f "$COMPLETED_DIR/phase-1.7-official-dual-platform-benchmark.md" ]]; then
    fail "P1.7 present in both active/ and completed/"
  fi
  pass "P1.7 is sole ACTIVE block"
elif [[ "$completed_p16" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "ROADMAP ACTIVE_BLOCK=NONE after P1.6 close"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "SSOT ACTIVE_BLOCK=NONE after P1.6 close"
  require_contains "$SSOT" 'P1_6_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.6 CLOSED"
  require_contains "$ROADMAP" 'P1_6_STATUS[[:space:]]*=[[:space:]]*CLOSED' "ROADMAP P1.6 CLOSED"
  require_contains "$COMPLETED_DIR/phase-1.6-release-candidate-freeze.md" 'P1_6_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.6 CLOSED"
  require_contains "$SSOT" 'P1_5_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.5 CLOSED after P1.6"
  require_contains "$SSOT" 'RC_STATUS[[:space:]]*=[[:space:]]*DECLARED_INTERNAL_NOT_RELEASED' "SSOT RC_STATUS=DECLARED_INTERNAL_NOT_RELEASED"
  require_contains "$ROADMAP" 'RC_STATUS[[:space:]]*=[[:space:]]*DECLARED_INTERNAL_NOT_RELEASED' "ROADMAP RC_STATUS=DECLARED_INTERNAL_NOT_RELEASED"
  require_contains "$COMPLETED_DIR/phase-1.6-release-candidate-freeze.md" 'RC_STATUS[[:space:]]*=[[:space:]]*DECLARED_INTERNAL_NOT_RELEASED' "completed P1.6 RC_STATUS"
  require_contains "$SSOT" 'P1_6_WS7[[:space:]]*=[[:space:]]*PASS' "SSOT P1_6_WS7=PASS"
  require_contains "$SSOT" 'READY_TO_OPEN_P1_7[[:space:]]*=[[:space:]]*YES' "SSOT READY_TO_OPEN_P1_7=YES"
  require_contains "$SSOT" 'AUTO_OPEN_P1_7[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_7=NO"
  require_contains "$SSOT" 'P1_7_OPENED[[:space:]]*=[[:space:]]*NO' "P1_7_OPENED=NO"
  require_contains "$SSOT" 'PUBLIC_RELEASE[[:space:]]*=[[:space:]]*NO' "PUBLIC_RELEASE=NO after P1.6 close"
  require_contains "$SSOT" 'PUBLICATION_STATUS[[:space:]]*=[[:space:]]*FORBIDDEN' "PUBLICATION_STATUS=FORBIDDEN"
  require_contains "$SSOT" 'OFFICIAL_BENCHMARK_BLOCK[[:space:]]*=[[:space:]]*P1\.7' "official bench remains P1.7"
  require_contains "$SSOT" 'PHASE_1_PRODUCT_CLOSED[[:space:]]*=[[:space:]]*NO' "Phase 1 not complete while P1.7 pending"
  if [[ "$completed_p15" -ne 1 ]]; then
    fail "P1.6 completed requires completed P1.5"
  fi
  pass "P1.6 closed via MOVE_ACTIVE_TO_COMPLETED"
elif [[ "$completed_p15" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "ROADMAP ACTIVE_BLOCK=NONE after P1.5 close"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "SSOT ACTIVE_BLOCK=NONE after P1.5 close"
  require_contains "$SSOT" 'P1_5_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.5 CLOSED"
  require_contains "$ROADMAP" 'P1_5_STATUS[[:space:]]*=[[:space:]]*CLOSED' "ROADMAP P1.5 CLOSED"
  require_contains "$COMPLETED_DIR/phase-1.5-security-soak-ops-release-hardening.md" 'P1_5_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.5 CLOSED"
  require_contains "$SSOT" 'AUTO_OPEN_P1_6[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_6=NO"
  require_contains "$SSOT" 'P1_6_OPENED[[:space:]]*=[[:space:]]*NO' "P1_6_OPENED=NO"
  if [[ "$completed_p14" -ne 1 ]]; then
    fail "P1.5 completed requires completed P1.4"
  fi
  pass "P1.5 closed via MOVE_ACTIVE_TO_COMPLETED"
elif [[ "$completed_p14" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "ROADMAP ACTIVE_BLOCK=NONE after P1.4 close"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "SSOT ACTIVE_BLOCK=NONE after P1.4 close"
  require_contains "$SSOT" 'P1_4_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.4 CLOSED"
  require_contains "$ROADMAP" 'P1_4_STATUS[[:space:]]*=[[:space:]]*CLOSED' "ROADMAP P1.4 CLOSED"
  require_contains "$COMPLETED_DIR/phase-1.4-configuration-product-completion.md" 'P1_4_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.4 CLOSED"
  require_contains "$SSOT" 'P1_3B_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.3b CLOSED after P1.4"
  require_contains "$SSOT" 'AUTO_OPEN_P1_5[[:space:]]*=[[:space:]]*NO' "AUTO_OPEN_P1_5=NO"
  require_contains "$SSOT" 'P1_5_OPENED[[:space:]]*=[[:space:]]*NO' "P1_5_OPENED=NO"
  if [[ "$completed_p13b" -ne 1 ]]; then
    fail "P1.4 completed requires completed P1.3b"
  fi
  pass "P1.4 closed via MOVE_ACTIVE_TO_COMPLETED"
elif [[ "$completed_p13b" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "ROADMAP ACTIVE_BLOCK=NONE after P1.3b close"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "SSOT ACTIVE_BLOCK=NONE after P1.3b close"
  require_contains "$SSOT" 'P1_3B_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.3b CLOSED"
  require_contains "$COMPLETED_DIR/phase-1.3b-http3-production-readiness.md" 'P1_3B_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.3b CLOSED"
  require_contains "$SSOT" 'P1_3A_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.3a CLOSED after P1.3b"
  pass "P1.3b closed via MOVE_ACTIVE_TO_COMPLETED"
elif [[ "$completed_p13a" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "ROADMAP ACTIVE_BLOCK=NONE after P1.3a close"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "SSOT ACTIVE_BLOCK=NONE after P1.3a close"
  require_contains "$SSOT" 'P1_3A_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.3a CLOSED"
  require_contains "$COMPLETED_DIR/phase-1.3a-proxy-tls-h2-product-completion.md" 'P1_3A_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.3a CLOSED"
  require_contains "$SSOT" 'P1_2_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.2 CLOSED after P1.3a"
  pass "P1.3a closed via MOVE_ACTIVE_TO_COMPLETED"
elif [[ "$completed_p12" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "ROADMAP ACTIVE_BLOCK=NONE after P1.2 close"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "SSOT ACTIVE_BLOCK=NONE after P1.2 close"
  require_contains "$SSOT" 'P1_2_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.2 CLOSED"
  require_contains "$COMPLETED_DIR/phase-1.2-fastcgi-php-fpm-production-grade.md" 'P1_2_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.2 CLOSED"
  require_contains "$SSOT" 'FASTCGI_PRODUCT_STATUS[[:space:]]*=[[:space:]]*PRODUCT_CLOSED' "FastCGI product closed after P1.2"
  pass "P1.2 closed via MOVE_ACTIVE_TO_COMPLETED"
elif [[ "$completed_p11" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "ROADMAP ACTIVE_BLOCK=NONE after P1.1 close"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "SSOT ACTIVE_BLOCK=NONE after P1.1 close"
  require_contains "$SSOT" 'P1_1_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.1 CLOSED"
  require_contains "$COMPLETED_DIR/phase-1.1-correctness-and-stability.md" 'P1_1_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.1 CLOSED"
  pass "P1.1 closed via MOVE_ACTIVE_TO_COMPLETED"
elif [[ "$completed_p10" -eq 1 ]]; then
  require_contains "$ROADMAP" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "ROADMAP ACTIVE_BLOCK=NONE after P1.0 close"
  require_contains "$SSOT" 'ACTIVE_BLOCK[[:space:]]*=[[:space:]]*NONE' "SSOT ACTIVE_BLOCK=NONE after P1.0 close"
  require_contains "$SSOT" 'P1_0_STATUS[[:space:]]*=[[:space:]]*CLOSED' "SSOT P1.0 CLOSED"
  require_contains "$COMPLETED_DIR/phase-1.0-canonical-state-refresh.md" 'P1_0_STATUS[[:space:]]*=[[:space:]]*CLOSED' "completed P1.0 CLOSED"
  pass "P1.0 closed; no later block active"
else
  fail "P1.0 document missing from both active/ and completed/"
fi

# Block index must list all Phase 1 blocks
for b in P1.1 P1.2 P1.3a P1.3b P1.4 P1.5 P1.6 P1.7; do
  if ! rg -q "$b" "$ROADMAP"; then
    fail "ROADMAP missing block $b"
  fi
done
# Successor blocks remain NOT_STARTED until authorized
if [[ "$active_p12" -eq 0 && "$completed_p12" -eq 0 ]]; then
  require_contains "$ROADMAP" 'P1\.2.*NOT_STARTED|NOT_STARTED.*P1\.2' "P1.2 NOT_STARTED"
fi
if [[ "$active_p13a" -eq 0 && "$completed_p13a" -eq 0 ]]; then
  require_contains "$ROADMAP" 'P1\.3a.*NOT_STARTED|NOT_STARTED.*P1\.3a' "P1.3a NOT_STARTED"
fi
if [[ "$active_p13b" -eq 0 && "$completed_p13b" -eq 0 ]]; then
  require_contains "$ROADMAP" 'P1\.3b.*NOT_STARTED|NOT_STARTED.*P1\.3b' "P1.3b NOT_STARTED"
fi
require_contains "$ROADMAP" 'ONE_ACTIVE_BLOCK[[:space:]]*=[[:space:]]*YES' "ONE_ACTIVE_BLOCK=YES"

# Broken-link check for local relative markdown links in roadmap core files
check_links() {
  local f="$1"
  local dir
  dir="$(dirname "$f")"
  # markdown links: [text](path) — skip http(s), mailto, anchors-only
  while IFS= read -r link; do
    [[ -z "$link" ]] && continue
    case "$link" in
      http://*|https://*|mailto:*|\#*) continue ;;
    esac
    local target="${link%%\#*}"
    [[ -z "$target" ]] && continue
    if [[ ! -e "$dir/$target" && ! -e "$ROOT/$target" ]]; then
      # also try relative to roadmap dir
      if [[ ! -e "$ROADMAP_DIR/$target" ]]; then
        fail "broken link in ${f#"$ROOT"/}: $link"
      fi
    fi
  done < <(rg -o '\[[^\]]*\]\(([^)]+)\)' -r '$1' "$f" || true)
}

check_links "$SSOT"
check_links "$ROADMAP"
check_links "$ARCHIVE_MR"
if [[ -f "$ACTIVE_DIR/phase-1.0-canonical-state-refresh.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.0-canonical-state-refresh.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.0-canonical-state-refresh.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.0-canonical-state-refresh.md"
fi
if [[ -f "$ACTIVE_DIR/phase-1.1-correctness-and-stability.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.1-correctness-and-stability.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.1-correctness-and-stability.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.1-correctness-and-stability.md"
fi
if [[ -f "$ACTIVE_DIR/phase-1.2-fastcgi-php-fpm-production-grade.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.2-fastcgi-php-fpm-production-grade.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.2-fastcgi-php-fpm-production-grade.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.2-fastcgi-php-fpm-production-grade.md"
fi
if [[ -f "$ACTIVE_DIR/phase-1.3a-proxy-tls-h2-product-completion.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.3a-proxy-tls-h2-product-completion.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.3a-proxy-tls-h2-product-completion.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.3a-proxy-tls-h2-product-completion.md"
fi
if [[ -f "$ACTIVE_DIR/phase-1.3b-http3-production-readiness.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.3b-http3-production-readiness.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.3b-http3-production-readiness.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.3b-http3-production-readiness.md"
fi
if [[ -f "$ACTIVE_DIR/phase-1.4-configuration-product-completion.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.4-configuration-product-completion.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.4-configuration-product-completion.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.4-configuration-product-completion.md"
fi
if [[ -f "$ACTIVE_DIR/phase-1.5-security-soak-ops-release-hardening.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.5-security-soak-ops-release-hardening.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.5-security-soak-ops-release-hardening.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.5-security-soak-ops-release-hardening.md"
fi
if [[ -f "$ACTIVE_DIR/phase-1.6-release-candidate-freeze.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.6-release-candidate-freeze.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.6-release-candidate-freeze.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.6-release-candidate-freeze.md"
fi
if [[ -f "$ACTIVE_DIR/phase-1.7-official-dual-platform-benchmark.md" ]]; then
  check_links "$ACTIVE_DIR/phase-1.7-official-dual-platform-benchmark.md"
fi
if [[ -f "$COMPLETED_DIR/phase-1.7-official-dual-platform-benchmark.md" ]]; then
  check_links "$COMPLETED_DIR/phase-1.7-official-dual-platform-benchmark.md"
fi

# Forbid markdown links to IDE-local plan trees from roadmap SSOT/ROADMAP
if rg -q '\[[^\]]*\]\([^)]*\.cursor/plans/[^)]*\)' "$SSOT"; then
  fail "SSOT must not markdown-link .cursor/plans as productive authority"
else
  pass "SSOT must not link .cursor/plans as productive authority"
fi
if rg -q '\[[^\]]*\]\([^)]*\.cursor/plans/[^)]*\)' "$ROADMAP"; then
  fail "ROADMAP must not markdown-link .cursor/plans as productive authority"
else
  pass "ROADMAP must not link .cursor/plans as productive authority"
fi

echo "verify-roadmap-consistency: summary errors=$errors"
if [[ "$errors" -gt 0 ]]; then
  echo "verify-roadmap-consistency: FAIL"
  exit 1
fi
echo "verify-roadmap-consistency: OK"
exit 0
