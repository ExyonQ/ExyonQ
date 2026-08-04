#!/usr/bin/env bash
# BASIC_PRODUCT_COMPLETENESS_GATE
# R3 may resume only when this gate PASS (owner: BASIC PRODUCT COMPLETENESS CLOSEOUT).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE="${1:---help}"

selftest() {
  echo "basic-product-completeness-gate selftest OK"
  exit 0
}

usage() {
  cat <<'EOF'
Usage:
  basic-product-completeness-gate.sh --selftest
  basic-product-completeness-gate.sh --check STATUS_FILE

STATUS_FILE must contain KEY=VALUE lines for required fields.
EOF
}

check_file() {
  local f="$1"
  [[ -f "$f" ]] || { echo "FAIL: missing $f"; exit 1; }
  local req=(
    STATIC_REAL_E2E
    PROXY_REAL_E2E
    FASTCGI_REAL_PHP_FPM_E2E
    TLS_REAL_E2E
    HTTP2_REAL_E2E
    HTTP3_REAL_E2E
    RELOAD_DRAIN_REAL_E2E
    OCI_AMD64_REAL_RUNTIME_E2E
    OCI_ARM64_REAL_RUNTIME_E2E
    NO_BASIC_PRODUCT_DEFECTS
    NO_UNJUSTIFIED_CONDITIONAL_BASIC_CAPABILITIES
    NO_UNJUSTIFIED_DEFERRED_BASIC_CAPABILITIES
    NO_AUTHORITATIVE_SMOKE_GATES
    HTTP3_POST_BODY_FIX_AMD64
    HTTP3_POST_BODY_FIX_ARM64
    R3_RESUME_ALLOWED
  )
  local missing=0
  for k in "${req[@]}"; do
    if ! grep -Eq "^${k}=" "$f"; then
      echo "FAIL: missing key $k"
      missing=1
    fi
  done
  [[ "$missing" -eq 0 ]] || exit 1

  fail=0
  for k in STATIC_REAL_E2E PROXY_REAL_E2E FASTCGI_REAL_PHP_FPM_E2E TLS_REAL_E2E \
           HTTP2_REAL_E2E HTTP3_REAL_E2E RELOAD_DRAIN_REAL_E2E \
           OCI_AMD64_REAL_RUNTIME_E2E OCI_ARM64_REAL_RUNTIME_E2E \
           HTTP3_POST_BODY_FIX_AMD64 HTTP3_POST_BODY_FIX_ARM64; do
    v="$(grep -E "^${k}=" "$f" | head -1 | cut -d= -f2- | tr -d '[:space:]')"
    if [[ "$v" != "PASS" && "$v" != "PASS_REAL_E2E" ]]; then
      echo "FAIL: $k=$v (need PASS/PASS_REAL_E2E)"
      fail=1
    fi
  done
  for k in NO_BASIC_PRODUCT_DEFECTS NO_UNJUSTIFIED_CONDITIONAL_BASIC_CAPABILITIES \
           NO_UNJUSTIFIED_DEFERRED_BASIC_CAPABILITIES NO_AUTHORITATIVE_SMOKE_GATES; do
    v="$(grep -E "^${k}=" "$f" | head -1 | cut -d= -f2- | tr -d '[:space:]')"
    if [[ "$v" != "YES" ]]; then
      echo "FAIL: $k=$v (need YES)"
      fail=1
    fi
  done
  # Gate itself never auto-sets R3_RESUME_ALLOWED=YES; it only checks consistency.
  r3="$(grep -E '^R3_RESUME_ALLOWED=' "$f" | head -1 | cut -d= -f2- | tr -d '[:space:]')"
  if [[ "$fail" -eq 0 && "$r3" != "YES" ]]; then
    echo "NOTE: completeness checks PASS but R3_RESUME_ALLOWED=$r3 (owner must flip)"
  fi
  if [[ "$fail" -ne 0 ]]; then
    echo "BASIC_PRODUCT_COMPLETENESS_GATE=FAIL"
    exit 1
  fi
  echo "BASIC_PRODUCT_COMPLETENESS_GATE=PASS"
}

case "$MODE" in
  --selftest) selftest ;;
  --check)
    [[ $# -ge 2 ]] || { usage; exit 2; }
    check_file "$2"
    ;;
  --help|-h) usage; exit 0 ;;
  *) usage; exit 2 ;;
esac
