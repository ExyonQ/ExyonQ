#!/usr/bin/env bash
# KD3 — Proxy boundary guards.
#
# baseline: KD3.2 — Hyper runtime in exyonq-mod-proxy; core uses ContractServiceSlot only.
# kd3_3: KD3.3 — shim reduction + ownership closure (Hyper conventional path out of core).
# closure: full KD3 extraction complete (future — see KD3_AUDIT_AND_BOUNDARY_PLAN.md).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

MODE="${1:-${KD3_PROXY_GUARD_MODE:-baseline}}"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify-kd3-proxy-guards: ripgrep required" >&2
  exit 2
fi

VIOLATIONS=0

fail() {
  VIOLATIONS=$((VIOLATIONS + 1))
  echo "verify-kd3-proxy-guards: VIOLATION: $*" >&2
}

pass() {
  echo "verify-kd3-proxy-guards: PASS: $*"
}

record() {
  echo "verify-kd3-proxy-guards: DEBT: $*"
}

# --- KD3.0 contract must exist ---
if [[ ! -f module-api/src/proxy_dispatch.rs ]]; then
  fail "module-api/src/proxy_dispatch.rs missing (KD3.0 contract)"
else
  pass "ProxyDispatchService contract present (KD3.0)"
fi

if rg -n 'trait ProxyDispatchService|ProxyCompiledSlot|ProxyStreamHandle' module-api/src/proxy_dispatch.rs >/dev/null 2>&1; then
  pass "proxy_dispatch defines core seam types"
else
  fail "proxy_dispatch missing required seam types"
fi

# --- Module crate ---
if [[ ! -d crates/exyonq-mod-proxy/src ]]; then
  fail "exyonq-mod-proxy crate required (KD3.2)"
else
  pass "exyonq-mod-proxy crate present"
fi

if rg -n 'exyonq-core|exyonq_core' crates/exyonq-mod-proxy/Cargo.toml >/dev/null 2>&1; then
  fail "exyonq-mod-proxy Cargo.toml must not list exyonq-core"
else
  pass "exyonq-mod-proxy Cargo.toml has no core dependency"
fi

# module-api proxy contract must not import hyper client/server internals
if rg -n 'hyper_util::client|hyper::client|Client<HttpConnector' module-api/src/proxy_dispatch.rs >/dev/null 2>&1; then
  fail "proxy_dispatch contract must not depend on Hyper client types"
else
  pass "proxy_dispatch contract is transport-agnostic"
fi

echo "verify-kd3-proxy-guards: MODE=$MODE"

if [[ "$MODE" == "baseline" ]]; then
  # KD3.2: Hyper runtime ownership lives in the module
  if rg -n 'hyper_util::client|build_hyper_client|HyperClientConfig' crates/exyonq-mod-proxy/src --type rust >/dev/null 2>&1; then
    pass "exyonq-mod-proxy owns Hyper client runtime (KD3.2)"
  else
    fail "exyonq-mod-proxy missing Hyper client runtime (KD3.2)"
  fi

  # Core must not build Hyper proxy clients (shims delegate only)
  core_hyper="$(rg -n 'Client::builder|build_hyper_client|HttpConnector::new|pool_max_idle_per_host' core/src --type rust 2>/dev/null \
    | rg -v 'core/src/proxy/mod.rs' || true)"
  if [[ -n "$core_hyper" ]]; then
    fail "core must not configure Hyper proxy clients (delegate to exyonq-mod-proxy)"
    echo "$core_hyper" >&2
  else
    pass "core has no Hyper proxy client configuration"
  fi

  # No legacy proxy setters/clears outside test-only registration gate
  legacy_setters="$(rg -n 'set_proxy_|clear_proxy_|PROXY_CLIENT|get_client\(\)' core/src --type rust 2>/dev/null \
    | rg -v 'clear_global_proxy_dispatch_for_register_once_test|ProxyDispatchTestGuard|reset_.*proxy_cache_metrics' || true)"
  if [[ -n "$legacy_setters" ]]; then
    fail "legacy proxy client setters/getters in core (use ContractServiceSlot + module)"
    echo "$legacy_setters" >&2
  else
    pass "no legacy proxy client setters in core"
  fi

  # execute_backend must wire PROXY_SERVICE (not inline Hyper policy)
  if rg -n 'PROXY_SERVICE|register_proxy_dispatch_service|dispatch_proxy' core/src/execute_backend.rs >/dev/null 2>&1; then
    pass "core proxy registration + dispatch seam wired (KD3.2)"
  else
    fail "execute_backend missing PROXY_SERVICE / dispatch_proxy (KD3.2)"
  fi

  proxy_policy_in_execute="$(rg -n 'forward_get|forward_request|build_incoming_client|UpstreamTarget::from' core/src/execute_backend.rs 2>/dev/null || true)"
  if [[ -n "$proxy_policy_in_execute" ]]; then
    fail "execute_backend must not embed Hyper proxy forward policy"
    echo "$proxy_policy_in_execute" >&2
  else
    pass "execute_backend has no Hyper proxy forward policy"
  fi

  # Inventory (remaining debt — cache frozen until KD3.5)
  for f in \
    crates/exyonq-mod-proxy/src/proxy_cache.rs \
    crates/exyonq-mod-proxy/src/cache_metrics.rs; do
    if [[ -f "$f" ]]; then
      loc="$(wc -l <"$f" | tr -d ' ')"
      record "$f ($loc LOC) — KD3.5 proxy cache ownership"
    fi
  done

  if [[ -f crates/exyonq-mod-proxy/src/wire_conn.rs ]]; then
    loc="$(wc -l <crates/exyonq-mod-proxy/src/wire_conn.rs | tr -d ' ')"
    record "crates/exyonq-mod-proxy/src/wire_conn.rs ($loc LOC) — KD3.4 wire transport"
  fi

  record "handler Hyper GET/POST via dispatch_proxy; wire GET /api/* via proxy_wire hooks"

  if rg -n 'tungstenite|tokio-tungstenite' core/Cargo.toml crates module-api Cargo.toml 2>/dev/null | rg -v 'verify-kd3-proxy-guards' >/dev/null 2>&1; then
    fail "WebSocket crate dependency added without KD3 ADR/guard waiver"
  else
    pass "no unauthorized tungstenite dependency"
  fi

  if rg -n 'fn proxy_.*use_blocking|EXYONQ_PROXY_.*BLOCKING|upstream_timeout_for_path' core/src/server/wire_dispatch.rs >/dev/null 2>&1; then
    matches="$(rg -n 'fn proxy_.*use_blocking|EXYONQ_PROXY_.*BLOCKING|upstream_timeout_for_path' core/src/server/wire_dispatch.rs 2>/dev/null || true)"
    fail "wire_dispatch must not duplicate proxy policy"
    echo "$matches" >&2
  else
    pass "no duplicated proxy policy in wire_dispatch"
  fi

  if [[ -f core/src/server/proxy_conn.rs ]]; then
    record "core/src/server/proxy_conn.rs still present (KD3.4 should move to mod-proxy)"
  else
    pass "proxy_conn absent from core (KD3.4+)"
  fi

  if [[ -f core/src/proxy/mod.rs ]]; then
    if rg -n 'KD3_TEMPORARY_SHIM' core/src/proxy/mod.rs >/dev/null 2>&1; then
      pass "core proxy/mod.rs marks KD3 temporary shims"
    else
      fail "core/src/proxy/mod.rs missing KD3_TEMPORARY_SHIM markers"
    fi
  else
    pass "core proxy shim module absent (KD3.5+)"
  fi

  dup_policy="$(rg -n 'remove\(hyper::header::CONNECTION\)|remove\(.*connection.*\)' core/src --type rust 2>/dev/null \
    | rg -v 'core/src/proxy/' || true)"
  if [[ -n "$dup_policy" ]]; then
    fail "duplicate header strip policy outside core/src/proxy (delegate to exyonq-mod-proxy)"
    echo "$dup_policy" >&2
  else
    pass "no duplicate hop-by-hop policy outside core/src/proxy"
  fi

  # Module: client pools via OnceLock OK; no service locator pattern
  if rg -n 'service_locator|static mut' crates/exyonq-mod-proxy/src --type rust >/dev/null 2>&1; then
    fail "exyonq-mod-proxy must not use service_locator/static mut"
  else
    pass "exyonq-mod-proxy has no service locator"
  fi

  # Block new proxy forward logic in wire_dispatch (frozen path)
  if rg -n 'forward_get|forward_request' core/src/server/wire_dispatch.rs >/dev/null 2>&1; then
    wire_proxy="$(rg -n 'forward_get|forward_request' core/src/server/wire_dispatch.rs 2>/dev/null || true)"
    fail "wire_dispatch must not embed proxy forward logic (frozen until KD3.4)"
    echo "$wire_proxy" >&2
  else
    pass "wire_dispatch has no proxy forward logic"
  fi

  # CLI must register proxy at bootstrap
  if rg -n 'register_proxy_dispatch_service|registration_with_default_runtime' cli/exyonq/src/main.rs >/dev/null 2>&1; then
    pass "CLI registers ProxyDispatchService at bootstrap"
  else
    fail "CLI missing proxy dispatch registration"
  fi

  # Accidental materialized streaming in execute_backend proxy path
  if rg -n 'ProxyDispatchOutcome::Streaming|collect\(\)\.await' core/src/execute_backend.rs >/dev/null 2>&1; then
    stream_in_execute="$(rg -n 'collect\(\)\.await' core/src/execute_backend.rs 2>/dev/null || true)"
    if [[ -n "$stream_in_execute" ]]; then
      fail "execute_backend must not materialize streaming proxy bodies"
      echo "$stream_in_execute" >&2
    else
      pass "execute_backend does not collect streaming proxy bodies"
    fi
  else
    pass "execute_backend proxy path avoids streaming materialization"
  fi

  if [[ "$VIOLATIONS" -eq 0 ]]; then
    echo "verify-kd3-proxy-guards: OK (baseline)"
    exit 0
  fi
  echo "verify-kd3-proxy-guards: FAIL ($VIOLATIONS violations)" >&2
  exit 1
fi

if [[ "$MODE" == "kd3_3" ]]; then
  # Run baseline checks first (shared invariants).
  KD3_PROXY_GUARD_MODE=baseline "$0" baseline || VIOLATIONS=$((VIOLATIONS + 1))

  # Spike proxy must live in module, not core.
  if rg -n 'run_spike_proxy' core/src --type rust >/dev/null 2>&1; then
    fail "run_spike_proxy must not remain in core (module-owned KD3.3)"
    rg -n 'run_spike_proxy' core/src --type rust >&2 || true
  else
    pass "run_spike_proxy removed from core"
  fi

  if rg -n 'pub async fn run_spike_proxy|mod spike' crates/exyonq-mod-proxy/src >/dev/null 2>&1; then
    pass "spike proxy owned by exyonq-mod-proxy"
  else
    fail "exyonq-mod-proxy missing spike proxy entry"
  fi

  if rg -n 'exyonq_mod_proxy::run_spike_proxy' cli/exyonq/src/main.rs >/dev/null 2>&1; then
    pass "CLI spike command uses exyonq-mod-proxy"
  else
    fail "CLI must invoke exyonq_mod_proxy::run_spike_proxy"
  fi

  # Allowed shims in core/src/proxy/mod.rs (allowlist) — absent after KD3.5.
  if [[ -f core/src/proxy/mod.rs ]]; then
    allowed_shims='forward_get_streaming'
    extra_shims="$(rg -n '^pub (async )?fn ' core/src/proxy/mod.rs 2>/dev/null \
      | rg -v "$allowed_shims" || true)"
    if [[ -n "$extra_shims" ]]; then
      fail "unexpected public shim functions in core/src/proxy/mod.rs"
      echo "$extra_shims" >&2
    else
      pass "core proxy shims match KD3.3 allowlist"
    fi
  else
    pass "core proxy shim module absent (KD3.5+)"
  fi

  if rg -n 'HyperClientConfig|set_hyper_client_config|build_hyper_client' core/src --type rust >/dev/null 2>&1; then
    fail "Hyper client config/factory must not appear in core"
    rg -n 'HyperClientConfig|set_hyper_client_config|build_hyper_client' core/src --type rust >&2 || true
  else
    pass "no HyperClientConfig/build_hyper_client in core"
  fi

  # Shims must delegate — no policy bodies in core proxy seam.
  if [[ -f core/src/proxy/mod.rs ]]; then
    for shim in forward_get forward_get_streaming forward_request build_client; do
      if ! rg -n "fn $shim" core/src/proxy/mod.rs >/dev/null 2>&1; then
        continue
      fi
      if rg -n "fn $shim" -A12 core/src/proxy/mod.rs 2>/dev/null | rg -n 'strip_hop_by_hop|Client::builder|pool_max_idle|UpstreamTarget::from_config' >/dev/null 2>&1; then
        fail "shim $shim contains policy in core (must delegate only)"
      fi
    done
    pass "core proxy shims are delegation-only"
  else
    pass "core proxy shims absent (KD3.5+)"
  fi

  # module-api must stay transport-agnostic
  if rg -n 'hyper::|hyper_util::' module-api/src/proxy_dispatch.rs >/dev/null 2>&1; then
    fail "module-api proxy_dispatch must not reference Hyper types"
  else
    pass "module-api proxy contract has no Hyper types"
  fi

  # Frozen paths — no premature wire/SSE/WS redesign
  if rg -n 'forward_get|forward_request' core/src/server/wire_dispatch.rs >/dev/null 2>&1; then
    fail "wire_dispatch proxy forward frozen until KD3.4"
  else
    pass "wire_dispatch unchanged (KD3.3)"
  fi

  if [[ ! -f core/tests/kd3_3_proxy_ownership_test.rs ]]; then
    fail "KD3.3 ownership tests missing"
  else
    pass "KD3.3 ownership tests present"
  fi

  if [[ "$VIOLATIONS" -eq 0 ]]; then
    echo "verify-kd3-proxy-guards: OK (kd3_3)"
    exit 0
  fi
  echo "verify-kd3-proxy-guards: FAIL ($VIOLATIONS violations)" >&2
  exit 1
fi

if [[ "$MODE" == "kd3_4" ]]; then
  KD3_PROXY_GUARD_MODE=baseline bash "$0" baseline || VIOLATIONS=$((VIOLATIONS + 1))

  if [[ -f core/src/server/proxy_conn.rs ]]; then
    fail "proxy_conn must not remain in core (KD3.4)"
  else
    pass "proxy_conn removed from core"
  fi

  if rg -n 'mod proxy_conn' core/src/server/mod.rs >/dev/null 2>&1; then
    fail "core must not mod proxy_conn (KD3.4)"
  else
    pass "core server/mod.rs has no proxy_conn module"
  fi

  if [[ ! -f crates/exyonq-mod-proxy/src/wire_conn.rs ]]; then
    fail "exyonq-mod-proxy missing wire_conn.rs"
  else
    pass "wire_conn owned by exyonq-mod-proxy"
  fi

  if [[ ! -f module-api/src/proxy_wire.rs ]]; then
    fail "module-api missing proxy_wire contract"
  else
    pass "proxy_wire contract present"
  fi

  if [[ -f core/src/proxy/mod.rs ]]; then
    if rg -n 'pub async fn forward_get[^_]|forward_request|build_client|build_incoming_client' core/src/proxy/mod.rs >/dev/null 2>&1; then
      fail "core/proxy/mod.rs must only retain cache streaming shim (KD3.5)"
      rg -n 'forward_get[^_]|forward_request|build_client' core/src/proxy/mod.rs >&2 || true
    else
      pass "core/proxy only forward_get_streaming shim"
    fi
  else
    pass "core/proxy shim module absent (KD3.5+)"
  fi

  if rg -n 'forward_get|forward_request|UpstreamTarget::from|strip_hop_by_hop' core/src/server/wire_dispatch.rs >/dev/null 2>&1; then
    fail "wire_dispatch must not embed proxy policy (KD3.4)"
    rg -n 'forward_get|forward_request|strip_hop_by_hop' core/src/server/wire_dispatch.rs >&2 || true
  else
    pass "wire_dispatch uses proxy_wire contract only"
  fi

  if rg -n 'mod websocket|forward_websocket|fn tunnel' core/src/proxy >/dev/null 2>&1; then
    fail "WebSocket FSM must not live in core/src/proxy"
  else
    pass "WebSocket FSM not in core/proxy"
  fi

  if rg -n 'forward_request' core/src/server/handler.rs >/dev/null 2>&1; then
    fail "handler must use exyonq_mod_proxy::forward_websocket not forward_request shim"
  else
    pass "handler WebSocket uses module forward_websocket"
  fi

  if rg -n 'exyonq-core|exyonq_core' crates/exyonq-mod-proxy/Cargo.toml >/dev/null 2>&1; then
    fail "exyonq-mod-proxy must not depend on exyonq-core"
  else
    pass "mod-proxy has no core dependency"
  fi

  if [[ ! -f core/tests/kd3_4_wire_proxy_test.rs ]]; then
    fail "KD3.4 wire tests missing"
  else
    pass "KD3.4 wire tests present"
  fi

  if [[ "$VIOLATIONS" -eq 0 ]]; then
    echo "verify-kd3-proxy-guards: OK (kd3_4)"
    exit 0
  fi
  echo "verify-kd3-proxy-guards: FAIL ($VIOLATIONS violations)" >&2
  exit 1
fi

if [[ "$MODE" == "kd3_5" ]]; then
  KD3_PROXY_GUARD_MODE=kd3_4 bash "$0" kd3_4 || VIOLATIONS=$((VIOLATIONS + 1))

  if [[ -f core/src/proxy/mod.rs ]]; then
    fail "core/src/proxy/mod.rs must be removed (KD3.5)"
  else
    pass "core proxy shim module removed"
  fi

  if [[ -f core/src/proxy_cache.rs ]]; then
    fail "core/src/proxy_cache.rs must move to exyonq-mod-proxy (KD3.5)"
  else
    pass "core proxy_cache removed"
  fi

  if rg -n 'forward_get_streaming|prepare_proxy_cache_load' core/src --type rust >/dev/null 2>&1; then
    fail "proxy streaming/cache load must not remain in core (KD3.5)"
    rg -n 'forward_get_streaming|prepare_proxy_cache_load' core/src --type rust >&2 || true
  else
    pass "no proxy streaming/cache load in core"
  fi

  if rg -n 'CACHE_PROXY_HITS|CACHE_PROXY_MISSES|CACHE_PROXY_INSERTIONS|CACHE_PROXY_REJECTIONS' core/src/cache.rs >/dev/null 2>&1; then
    fail "proxy cache metrics atomics must live in exyonq-mod-proxy (KD3.5)"
  else
    pass "proxy cache metrics not in core/cache.rs"
  fi

  if rg -n 'assess_cacheability|request_eligible_for_cache' core/src/proxy core/src/proxy_cache.rs 2>/dev/null; then
    fail "proxy cacheability policy must not live in core (KD3.5)"
  else
    pass "no proxy cacheability policy in core proxy paths"
  fi

  if rg -n 'ProxyCacheLoad|load_get_for_cache|prepare_proxy_cache_load' crates/exyonq-mod-proxy/src/proxy_cache.rs >/dev/null 2>&1; then
    pass "exyonq-mod-proxy owns proxy_cache materialization"
  else
    fail "exyonq-mod-proxy missing proxy_cache.rs"
  fi

  if rg -n 'note_proxy_hit|cache_proxy_hits_total' crates/exyonq-mod-proxy/src/cache_metrics.rs >/dev/null 2>&1; then
    pass "exyonq-mod-proxy owns proxy cache metrics"
  else
    fail "exyonq-mod-proxy missing cache_metrics.rs"
  fi

  if rg -n 'ProxyStreamHandle|classify_hyper_response|store_streaming_response' crates/exyonq-mod-proxy/src >/dev/null 2>&1; then
    pass "ProxyStreamHandle streaming path in module"
  else
    fail "module missing streaming handle wiring"
  fi

  if rg -n 'hyper::|hyper_util::|tokio::' module-api/src/proxy_dispatch.rs >/dev/null 2>&1; then
    fail "module-api proxy_dispatch must not reference Hyper/Tokio types (KD3.5)"
  else
    pass "module-api proxy contract transport-agnostic (KD3.5)"
  fi

  if rg -n 'mod proxy|pub mod proxy' core/src/lib.rs >/dev/null 2>&1; then
    fail "core must not export proxy module (KD3.5)"
  else
    pass "core lib.rs has no proxy module"
  fi

  if [[ "$VIOLATIONS" -eq 0 ]]; then
    echo "verify-kd3-proxy-guards: OK (kd3_5)"
    exit 0
  fi
  echo "verify-kd3-proxy-guards: FAIL ($VIOLATIONS violations)" >&2
  exit 1
fi

if [[ "$MODE" == "closure" ]]; then
  KD3_PROXY_GUARD_MODE=kd3_5 bash "$0" kd3_5 || VIOLATIONS=$((VIOLATIONS + 1))

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

  check_absent "core must not define UpstreamTarget" 'struct UpstreamTarget' core/src
  check_absent "core must not define ProxyClient pool type" 'type ProxyClient' core/src
  check_absent "core must not call forward_get from handler" 'forward_get[^_]' core/src/server/handler.rs
  check_absent "proxy_conn wire loop must be module-owned" 'mod proxy_conn' core/src/server/mod.rs
  check_absent "snapshot must not store UpstreamTarget runtime" 'UpstreamTarget' crates/exyonq-runtime-plan/src/runtime_plan.rs
  check_absent "ServerState must not cache upstream_targets" 'upstream_targets' core/src/server/state.rs
  check_absent "core must not retain proxy_slots runtime table" 'proxy_slots' core/src
  check_absent "KD3 temporary shims must be absent at closure" 'KD3_TEMPORARY_SHIM' core/src
  check_absent "core must not wildcard-reexport proxy module" 'pub use exyonq_mod_proxy::\*' core/src

  if rg -n 'UpstreamTarget' core/src/server/handler.rs >/dev/null 2>&1; then
    fail "handler must not reference UpstreamTarget (KD3.6 slot-only)"
    rg -n 'UpstreamTarget' core/src/server/handler.rs >&2 || true
  else
    pass "handler has no UpstreamTarget references"
  fi

  if rg -n 'load_get_for_cache[^_]|forward_websocket[^_]' core/src/server/handler.rs >/dev/null 2>&1; then
    fail "handler must use cluster_id module APIs (load_get_for_cache_by_cluster)"
    rg -n 'load_get_for_cache[^_]|forward_websocket[^_]' core/src/server/handler.rs >&2 || true
  else
    pass "handler uses cluster_id proxy module APIs"
  fi

  if rg -n 'proxy_compiled_slots:' crates/exyonq-runtime-plan/src/runtime_plan.rs >/dev/null 2>&1; then
    pass "snapshot stores ProxyCompiledSlot metadata (KD3.6)"
  else
    fail "snapshot missing proxy_compiled_slots field"
  fi

  if rg -n 'exyonq-core|exyonq_core' crates/exyonq-mod-proxy/Cargo.toml >/dev/null 2>&1; then
    fail "exyonq-mod-proxy must not depend on exyonq-core"
  else
    pass "mod-proxy has no core dependency (KD3.6)"
  fi

  if rg -n 'hyper::|hyper_util::|tokio::|mio::|epoll' module-api/src/proxy_dispatch.rs module-api/src/proxy_wire.rs >/dev/null 2>&1; then
    fail "module-api proxy contracts must not leak Hyper/Tokio/epoll"
  else
    pass "module-api proxy contracts transport-agnostic (KD3.6)"
  fi

  if ! rg -n 'PROXY_SERVICE|register_proxy_dispatch_service' core/src/execute_backend.rs >/dev/null 2>&1; then
    fail "core missing generic proxy registration seam at closure"
  else
    pass "core uses ContractServiceSlot for proxy registration"
  fi

  if [[ "$VIOLATIONS" -eq 0 ]]; then
    echo "verify-kd3-proxy-guards: OK (closure)"
    exit 0
  fi
  echo "verify-kd3-proxy-guards: FAIL ($VIOLATIONS violations)" >&2
  exit 1
fi

echo "verify-kd3-proxy-guards: unknown mode '$MODE' (use baseline|kd3_3|kd3_4|kd3_5|closure)" >&2
exit 2
