#!/usr/bin/env bash
# KD2.5 — Static boundary guards (closure mode default).
#
# Closure: core must not retain temporary shims, static policy, or module internals.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

MODE="${KD2_STATIC_GUARD_MODE:-closure}"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify-kd2-static-guards: ripgrep required" >&2
  exit 2
fi

VIOLATIONS=0

fail() {
  VIOLATIONS=$((VIOLATIONS + 1))
  echo "verify-kd2-static-guards: VIOLATION: $*" >&2
}

pass() {
  echo "verify-kd2-static-guards: PASS: $*"
}

check_absent() {
  local label="$1"
  shift
  local matches
  matches="$(rg -n "$@" --type rust 2>/dev/null || true)"
  if [[ -n "$matches" ]]; then
    fail "$label"
    echo "$matches" >&2
  else
    pass "$label"
  fi
}

# --- Contract + module ownership (all modes) ---
if [[ ! -f module-api/src/static_dispatch.rs ]]; then
  fail "module-api/src/static_dispatch.rs missing (KD2 contract)"
else
  pass "StaticDispatchService contract present"
fi

if rg -n 'StaticCompiledSlot' module-api/src/static_dispatch.rs >/dev/null 2>&1; then
  pass "StaticCompiledSlot defined in module-api (KD2.5)"
else
  fail "StaticCompiledSlot missing from module-api"
fi

if [[ ! -d crates/exyonq-mod-static/src ]]; then
  fail "crates/exyonq-mod-static missing"
else
  pass "exyonq-mod-static crate present"
fi

if rg -n 'struct StaticRoot|impl StaticRoot' crates/exyonq-mod-static/src --type rust >/dev/null 2>&1; then
  pass "StaticRoot owned by exyonq-mod-static"
else
  fail "exyonq-mod-static missing StaticRoot ownership"
fi

if rg -n 'struct StaticRuntime|impl StaticDispatchService' crates/exyonq-mod-static/src --type rust >/dev/null 2>&1; then
  pass "StaticRuntime + StaticDispatchService owned by exyonq-mod-static"
else
  fail "exyonq-mod-static missing StaticRuntime / StaticDispatchService"
fi

# --- exyonq-mod-static must not depend on core ---
if rg -n 'exyonq_core|exyonq-core' crates/exyonq-mod-static --type rust 2>/dev/null | rg -v 'Cargo.toml' >/dev/null 2>&1; then
  fail "exyonq-mod-static must not depend on exyonq-core"
else
  pass "exyonq-mod-static does not import exyonq-core"
fi

FORBIDDEN_MOD_STATIC=(
  'crate::server::'
  'core::server::'
  'wire_dispatch'
  'epoll_worker'
  'snapshot::'
  'crate::cache::'
)
for pat in "${FORBIDDEN_MOD_STATIC[@]}"; do
  if matches="$(rg -n "$pat" crates/exyonq-mod-static/src --type rust --glob '!*_tests.rs' 2>/dev/null || true)"; [[ -n "$matches" ]]; then
    fail "exyonq-mod-static forbidden import: $pat"
    echo "$matches" >&2
  fi
done
pass "exyonq-mod-static avoids server/epoll/wire/snapshot/cache internals"

# --- KD2.4 cache boundary ---
if [[ -f crates/exyonq-mod-static/src/static_cache.rs ]]; then
  pass "static_cache policy module present"
else
  fail "missing crates/exyonq-mod-static/src/static_cache.rs"
fi

if rg -n '\.matches_current\(' core/src/cache.rs >/dev/null 2>&1; then
  fail "core/cache.rs still calls static identity matches_current"
else
  pass "core/cache.rs delegates static revalidation"
fi

if rg -n 'CACHE_STATIC_' core/src/cache.rs >/dev/null 2>&1; then
  fail "core/cache.rs still owns CACHE_STATIC_* counters"
else
  pass "core/cache.rs has no CACHE_STATIC_* counters"
fi

if rg -n 'STATIC_SERVICE|register_static_dispatch_service' core/src/execute_backend.rs >/dev/null 2>&1; then
  pass "core uses ContractServiceSlot for static registration"
else
  fail "core missing generic static registration seam"
fi

if rg -n 'resolve_path_uncached|serve_request\(' core/src/server/handler.rs >/dev/null 2>&1; then
  fail "core handler still owns static resolution logic"
else
  pass "core handler does not call StaticRoot serve_request directly"
fi

if [[ "$MODE" == "baseline" ]]; then
  echo "verify-kd2-static-guards: MODE=baseline (legacy KD2.1–KD2.4 checks skipped — use closure)"
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    echo "verify-kd2-static-guards: FAIL ($VIOLATIONS violations)" >&2
    exit 1
  fi
  echo "verify-kd2-static-guards: OK (baseline)"
  exit 0
fi

echo "verify-kd2-static-guards: MODE=closure (KD2.5)"

# --- KD2.5: deleted shim paths must not exist ---
for path in \
  core/src/static_files/mod.rs \
  core/src/static_resource_identity.rs \
  core/src/server/static_conn.rs \
  core/src/server/epoll_sendfile.rs \
  core/src/server/epoll_sendfile_metrics.rs; do
  if [[ -e "$path" ]]; then
    fail "KD2.5 shim file must be deleted: $path"
  else
    pass "shim absent: $path"
  fi
done

if [[ -d core/src/static_files ]]; then
  fail "core/src/static_files directory must not exist"
else
  pass "core/src/static_files directory absent"
fi

# --- KD2.5: no temporary markers or wildcard reexports ---
check_absent "no KD2_TEMPORARY_SHIM in core" 'KD2_TEMPORARY_SHIM' core/src
check_absent "no pub use exyonq_mod_static::* in core" 'pub use exyonq_mod_static::\*' core/src
mod_static_prod_ok=1
KD41_STATIC_ALLOW='core/src/server/handler\.rs|core/src/lib\.rs|core/src/execute_backend\.rs'
for f in $(rg -l 'exyonq_mod_static' core/src --type rust 2>/dev/null || true); do
  if echo "$f" | rg -q "$KD41_STATIC_ALLOW"; then
    pass "exyonq_mod_static allowed in KD4.1 cache seam: $f"
    continue
  fi
  test_line="$(rg -n '^(#\[cfg.*\]\s*)?mod (tests|fsm_wiring_tests)\b' "$f" 2>/dev/null | head -1 | cut -d: -f1 || true)"
  if [[ -z "$test_line" ]]; then
    matches="$(rg -n 'exyonq_mod_static' "$f" --type rust 2>/dev/null || true)"
    if [[ -n "$matches" ]]; then
      mod_static_prod_ok=0
      fail "exyonq_mod_static outside test module in $f"
      echo "$matches" >&2
    fi
    continue
  fi
  prod_matches="$(rg -n 'exyonq_mod_static' "$f" --type rust 2>/dev/null | awk -F: -v t="$test_line" '$1+0 < t+0' || true)"
  if [[ -n "$prod_matches" ]]; then
    mod_static_prod_ok=0
    fail "exyonq_mod_static referenced outside test-only cfg in $f"
    echo "$prod_matches" >&2
  fi
done
if [[ "$mod_static_prod_ok" -eq 1 ]]; then
  pass "exyonq_mod_static absent from core production code"
fi

# --- KD2.5: core must not define static operational types ---
check_absent "core must not define StaticRoot" 'struct StaticRoot|impl StaticRoot' core/src
check_absent "core must not define StaticResourceIdentity" 'struct StaticResourceIdentity|impl StaticResourceIdentity' core/src
check_absent "core must not define sendfile FSM" 'struct SendingState|enum PumpResult|fn pump_sendfile_nb' core/src
check_absent "core must not call libc::sendfile" 'sendfile64|libc::sendfile' core/src
check_absent "core must not own precooked wire responses" 'PrecookedResponse|WirePair|bench_header_keep' core/src

# --- KD2.5: snapshot slot-only (no operational root in snapshot) ---
if rg -n 'site_static[^_slot]|Arc<StaticRoot>|fn static_root_for_' core/src/snapshot --type rust 2>/dev/null | rg -v 'site_static_slot|site_static_root_slot' >/dev/null 2>&1; then
  matches="$(rg -n 'site_static[^_slot]|Arc<StaticRoot>|fn static_root_for_' core/src/snapshot --type rust 2>/dev/null | rg -v 'site_static_slot|site_static_root_slot' || true)"
  fail "snapshot still carries operational static root APIs"
  echo "$matches" >&2
else
  pass "snapshot is slot-only (StaticCompiledSlot / site_static_slot)"
fi

if rg -n 'StaticCompiledSlot' crates/exyonq-runtime-plan/src/runtime_plan.rs >/dev/null 2>&1; then
  pass "snapshot uses StaticCompiledSlot from module-api"
else
  fail "snapshot missing StaticCompiledSlot"
fi

# --- KD2.5: core uses module-api hooks, not serve_blocking_sync implementation ---
if rg -n 'fn serve_blocking_sync' core/src --type rust >/dev/null 2>&1; then
  fail "core must not implement serve_blocking_sync (use module-api hook)"
else
  pass "serve_blocking_sync not implemented in core"
fi

if rg -n 'exyonq_module_api::static_wire|exyonq_module_api::static_epoll|exyonq_module_api::static_dispatch' core/src --type rust >/dev/null 2>&1; then
  pass "core uses module-api static contracts"
else
  fail "core missing module-api static contract imports"
fi

# --- KD2D: no duplicated blocking-pool policy in core wire_dispatch ---
if rg -n 'fn static_(wire_)?use_blocking_pool|fn static_sendfile_use_blocking_pool|fn static_one_m_sendfile_use_blocking|EXYONQ_(STATIC|SENDFILE).*BLOCKING' core/src/server/wire_dispatch.rs >/dev/null 2>&1; then
  matches="$(rg -n 'fn static_(wire_)?use_blocking_pool|fn static_sendfile_use_blocking_pool|fn static_one_m_sendfile_use_blocking|EXYONQ_(STATIC|SENDFILE).*BLOCKING' core/src/server/wire_dispatch.rs 2>/dev/null || true)"
  fail "core wire_dispatch must not duplicate static blocking-pool policy"
  echo "$matches" >&2
else
  pass "no duplicated blocking-pool policy in core wire_dispatch"
fi

# --- KD2D.1: no duplicated blocking-pool policy in platform workers (PS3A moved) ---
for f in \
  crates/exyonq-platform-linux/src/epoll_worker.rs \
  crates/exyonq-platform-linux/src/sync_accept.rs \
  crates/exyonq-platform-linux/src/io_uring_worker.rs; do
  if [[ ! -f "$f" ]]; then
    fail "expected platform worker file $f (PS3A ownership)"
  fi
  if rg -n 'fn static_(wire_)?use_blocking_pool|fn static_sendfile_use_blocking_pool|fn static_one_m_sendfile_use_blocking|EXYONQ_(STATIC|SENDFILE).*BLOCKING' "$f" >/dev/null 2>&1; then
    fail "platform $(basename "$f") must delegate blocking policy via module-api hooks only"
  else
    pass "no duplicated blocking-pool policy in platform $(basename "$f")"
  fi
done

# Policy lives in core executor / wire_dispatch after PM1 (platform only serves via facade).
if ! rg -n 'static_use_blocking_pool' core/src/server/connection_executor.rs core/src/server/wire_dispatch.rs >/dev/null 2>&1; then
  fail "core must retain static_wire::static_use_blocking_pool for sync bench admission policy"
else
  pass "sync blocking eligibility remains core policy via module-api"
fi

if ! rg -n 'static_tcp_blocking_pool_admission' core/src/server/wire_dispatch.rs >/dev/null 2>&1; then
  fail "wire_dispatch must use static_tcp_blocking_pool_admission for TCP admission"
else
  pass "wire_dispatch delegates TCP blocking admission to module-api"
fi

# --- KD2D.1: no legacy static registry reset/setter APIs ---
if rg -n 'clear_static_dispatch_service_for_tests|set_static_dispatch_service_for_tests' core/src >/dev/null 2>&1; then
  matches="$(rg -n 'clear_static_dispatch_service_for_tests|set_static_dispatch_service_for_tests' core/src 2>/dev/null || true)"
  fail "legacy static registry reset APIs must not remain (use StaticDispatchTestGuard)"
  echo "$matches" >&2
else
  pass "no legacy static registry reset APIs"
fi

# --- KD2D.1: no legacy FastCGI registry setter APIs (use FcgiDispatchTestGuard) ---
if rg -n 'clear_fcgi_executor_for_tests|set_fcgi_dispatch_service_for_tests|set_fcgi_executor_for_tests|set_fcgi_runtime_for_tests|set_fcgi_delegate_timeout_for_tests' core/src >/dev/null 2>&1; then
  matches="$(rg -n 'clear_fcgi_executor_for_tests|set_fcgi_dispatch_service_for_tests|set_fcgi_executor_for_tests|set_fcgi_runtime_for_tests|set_fcgi_delegate_timeout_for_tests' core/src 2>/dev/null || true)"
  fail "legacy FastCGI registry setter APIs must not remain (use FcgiDispatchTestGuard)"
  echo "$matches" >&2
else
  pass "no legacy FastCGI registry setter APIs"
fi

# --- KD4.1: core may depend on exyonq-mod-static for cache serve orchestration ---
if awk '/^\[dependencies\]/{d=1} /^\[/{if($0!="[dependencies]")d=0} d&&/exyonq-mod-static/' core/Cargo.toml | rg -q .; then
  pass "exyonq-mod-static in core production deps (KD4.1 cache serve)"
elif rg -n 'exyonq-mod-static' core/Cargo.toml >/dev/null 2>&1; then
  if rg -n 'dev-dependencies' -A20 core/Cargo.toml | rg -n 'exyonq-mod-static' >/dev/null 2>&1; then
    pass "exyonq-mod-static only in dev-dependencies for tests"
  fi
else
  pass "exyonq-mod-static not in core dependencies"
fi

# --- KD2.5: wire_conn + sendfile FSM in module ---
if [[ -f crates/exyonq-mod-static/src/wire_conn.rs ]] && [[ -f crates/exyonq-mod-static/src/sendfile_fsm.rs ]]; then
  pass "wire_conn + sendfile_fsm owned by exyonq-mod-static"
else
  fail "wire_conn or sendfile_fsm missing from exyonq-mod-static"
fi

if [[ -f crates/exyonq-mod-static/src/kernel_hooks.rs ]]; then
  pass "kernel_hooks bootstrap present in exyonq-mod-static"
else
  fail "missing install_kernel_hooks in exyonq-mod-static"
fi

# --- test-utils wiring ---
if ! rg -n 'test-utils' crates/exyonq-mod-static/Cargo.toml >/dev/null 2>&1; then
  fail "exyonq-mod-static must declare test-utils feature"
elif ! rg -n 'features = \["test-utils"\]' core/Cargo.toml >/dev/null 2>&1; then
  fail "exyonq-core dev-dependencies must enable exyonq-mod-static/test-utils"
else
  pass "test-utils feature wired for cross-crate test helper"
fi

if [[ "$VIOLATIONS" -gt 0 ]]; then
  echo "verify-kd2-static-guards: FAIL ($VIOLATIONS violations)" >&2
  exit 1
fi

echo "verify-kd2-static-guards: OK (closure)"
