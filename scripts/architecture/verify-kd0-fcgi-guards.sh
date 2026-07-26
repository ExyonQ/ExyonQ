#!/usr/bin/env bash
# KD0/KD1 — FastCGI boundary guards: core must not own module transport/runtime internals.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify-kd0-fcgi-guards: ripgrep required" >&2
  exit 2
fi

VIOLATIONS=0

fail() {
  VIOLATIONS=$((VIOLATIONS + 1))
  echo "verify-kd0-fcgi-guards: VIOLATION: $*" >&2
}

pass() {
  echo "verify-kd0-fcgi-guards: PASS: $*"
}

check_absent_in_core() {
  local label="$1"
  shift
  local matches
  matches="$(rg -n "$@" core/src --type rust 2>/dev/null || true)"
  if [[ -n "$matches" ]]; then
    fail "$label"
    echo "$matches" >&2
  else
    pass "$label"
  fi
}

check_absent_in_core "core must not reference FastCGI record types" \
  'FCGI_BEGIN_REQUEST|FCGI_PARAMS|FCGI_STDIN|FCGI_STDOUT|encode_record_frame|parse_record'

check_absent_in_core "core must not own FastCGI pool semaphores" \
  'Semaphore::new|try_acquire_owned|OwnedSemaphorePermit'

check_absent_in_core "core must not implement FastCGI status metric notes" \
  'note_fcgi_response_|note_fcgi_saturation|note_fcgi_inflight'

check_absent_in_core "core must not connect unix/tcp for FastCGI transport" \
  'UnixFpmTransport|WireTransport|connect_unix_stream|PhpFpmClient'

# KD4.1: cache serve orchestration may import mod-fastcgi from handler only.
fcgi_prod_imports="$(rg -n 'exyonq_mod_fastcgi' core/src --type rust 2>/dev/null \
  | rg -v 'core/src/server/handler\.rs|core/src/lib\.rs' || true)"
if [[ -n "$fcgi_prod_imports" ]]; then
  fail "core must not import exyonq-mod-fastcgi outside handler/lib cache seams (KD4.1)"
  echo "$fcgi_prod_imports" >&2
else
  if rg -n 'exyonq_mod_fastcgi' core/src/server/handler.rs >/dev/null 2>&1; then
    pass "core handler imports exyonq-mod-fastcgi for cache serve only (KD4.1)"
  else
    pass "core does not import exyonq-mod-fastcgi"
  fi
fi

if [[ -f core/src/fcgi_metrics.rs ]]; then
  fail "core/src/fcgi_metrics.rs must be removed (module-owned metrics)"
else
  pass "fcgi_metrics removed from core"
fi

if rg -n 'struct FcgiRuntimeState|run_fcgi_delegate|map_dispatch_outcome' core/src --type rust >/dev/null 2>&1; then
  fail "FastCGI runtime state still in core"
  rg -n 'struct FcgiRuntimeState|run_fcgi_delegate|map_dispatch_outcome' core/src --type rust >&2 || true
else
  pass "FastCGI runtime orchestration not in core"
fi

if rg -n 'FcgiDispatchService' core/src --type rust >/dev/null 2>&1; then
  pass "core uses generic FcgiDispatchService contract"
else
  fail "core missing FcgiDispatchService registration seam"
fi

if rg -n 'pub struct FcgiRuntime|pub mod runtime' crates/exyonq-mod-fastcgi/src --type rust >/dev/null 2>&1; then
  pass "FastCGI runtime owned by exyonq-mod-fastcgi"
else
  fail "exyonq-mod-fastcgi missing runtime ownership"
fi

if [[ "$VIOLATIONS" -gt 0 ]]; then
  echo "verify-kd0-fcgi-guards: FAIL ($VIOLATIONS violations)" >&2
  exit 1
fi

echo "verify-kd0-fcgi-guards: OK"
exit 0
