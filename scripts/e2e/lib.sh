# Shared helpers for scripts/e2e/*
# shellcheck shell=bash
set -euo pipefail

E2E_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$E2E_ROOT"

EXYONQ_BIN="${EXYONQ_BIN:-$E2E_ROOT/target/debug/exyonq}"
EXYONQCTL_BIN="${EXYONQCTL_BIN:-$E2E_ROOT/target/debug/exyonqctl}"

e2e_require_linux() {
  if [[ "$(uname -s)" != "Linux" ]]; then
    # NOT_APPLICABLE — must not report success (exit 0) on Darwin/local iteration.
    echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED platform=$(uname -s) LOCAL_ITERATION_ONLY"
    exit 2
  fi
}

e2e_ensure_bins() {
  if [[ ! -x "$EXYONQ_BIN" ]]; then
    cargo build -q -p exyonq --bin exyonq
  fi
  if [[ ! -x "$EXYONQCTL_BIN" ]]; then
    cargo build -q -p exyonqctl --bin exyonqctl 2>/dev/null \
      || cargo build -q -p exyonq --bin exyonqctl 2>/dev/null \
      || true
  fi
  [[ -x "$EXYONQ_BIN" ]] || { echo "FAIL: missing $EXYONQ_BIN"; return 1; }
}

e2e_pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

e2e_pass=0
e2e_fail=0
e2e_record_pass() { echo "PASS  $1"; e2e_pass=$((e2e_pass + 1)); }
e2e_record_fail() { echo "FAIL  $1"; e2e_fail=$((e2e_fail + 1)); }
e2e_finish() {
  local name="$1"
  echo "${name}: PASS=${e2e_pass} FAIL=${e2e_fail}"
  [[ "$e2e_fail" -eq 0 ]] || return 1
  [[ "$e2e_pass" -gt 0 ]] || {
    echo "FAIL: ${name} recorded zero PASS assertions"
    return 1
  }
}
