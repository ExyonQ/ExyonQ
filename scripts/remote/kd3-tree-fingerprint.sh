#!/usr/bin/env bash
# Deterministic KD3.2 tree fingerprint (local or remote workspace).
# Output: key=value lines + fingerprint=<sha256> on last line.
set -euo pipefail

if command -v sha256sum >/dev/null 2>&1; then
  SHA256() { sha256sum | awk '{print $1}'; }
else
  SHA256() { shasum -a 256 | awk '{print $1}'; }
fi

WORKSPACE="${1:-.}"
cd "$WORKSPACE"

MANIFEST="${KD32_SYNC_MANIFEST:-.kd32-sync-manifest}"
if [[ -f "$MANIFEST" ]]; then
  # shellcheck disable=SC1090
  source "$MANIFEST"
  head_sha="${KD32_HEAD:-unknown}"
  dirty_lines="${KD32_DIRTY_LINES:-0}"
  status_hash="${KD32_STATUS_HASH:-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855}"
else
  head_sha="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
  dirty_lines="$(git status --short 2>/dev/null | wc -l | tr -d ' ')"
  if [[ -z "$dirty_lines" ]]; then
    dirty_lines=0
  fi
  status_hash="$(
    git status --short 2>/dev/null | LC_ALL=C sort | SHA256
  )"
  if [[ -z "$status_hash" ]]; then
    status_hash=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
  fi
fi

bundle_hash="$(
  find \
    core/src/execute_backend.rs \
    core/src/http_cache.rs \
    core/src/lib.rs \
    core/src/http3_runtime_registry.rs \
    core/src/pipeline_registry.rs \
    core/src/htaccess_runtime_registry.rs \
    core/src/structural_route_rules.rs \
    crates/exyonq-mod-tls/src \
    crates/exyonq-mod-http3/src \
    crates/exyonq-module-pipeline/src \
    module-api/src/tls_runtime.rs \
    module-api/src/http3_runtime.rs \
    module-api/src/cross_cutting_pipeline.rs \
    module-api/src/route_rules.rs \
    core/src/contract_backend_http_metrics.rs \
    core/src/server/handler.rs \
    core/src/server/state.rs \
    core/src/snapshot/mod.rs \
    core/src/server/wire_dispatch.rs \
    crates/exyonq-cache/src \
    crates/exyonq-mod-static/src/cache_serve.rs \
    crates/exyonq-mod-proxy/src/cache_serve.rs \
    crates/exyonq-mod-fastcgi/src/cache_serve.rs \
    crates/exyonq-mod-fastcgi/src/cache_metrics.rs \
    crates/exyonq-mod-fastcgi/src/bridge.rs \
    module-api/src/proxy_dispatch.rs \
    crates/exyonq-mod-proxy/src \
    cli/exyonq/src/main.rs \
    core/tests/kd3_proxy_dispatch_test.rs \
    core/tests/kd3_3_proxy_ownership_test.rs \
    crates/exyonq-mod-proxy/src/proxy_cache.rs \
    crates/exyonq-mod-proxy/src/cache_metrics.rs \
    crates/exyonq-mod-proxy/src/wire_conn.rs \
    crates/exyonq-mod-proxy/src/wire_io.rs \
    crates/exyonq-mod-proxy/src/kernel_hooks.rs \
    module-api/src/proxy_wire.rs \
    core/tests/kd3_4_wire_proxy_test.rs \
    core/tests/plan12_cache_concurrency_test.rs \
    core/tests/plan12_cache_invalidation_test.rs \
    core/tests/plan12_proxy_cache_test.rs \
    scripts/architecture/verify-kd3-proxy-guards.sh \
    scripts/architecture/verify-kd4-core-residual-guards.sh \
    scripts/smoke/kd3-proxy-smoke.sh \
    crates/exyonq-mod-proxy/src/websocket.rs \
    scripts/smoke/kd34-ws-debug.sh \
    scripts/remote/kd34-ws-validation-matrix.sh \
    scripts/remote/kd34-smoke-validation-matrix.sh \
    scripts/remote/validate-kd3-native.sh \
    scripts/remote/kd3-tree-fingerprint.sh \
    -type f 2>/dev/null | LC_ALL=C sort | xargs shasum -a 256 2>/dev/null | SHA256
)"

fingerprint="$(printf '%s\n%s\n%s\n%s' "$head_sha" "$dirty_lines" "$status_hash" "$bundle_hash" | SHA256)"

echo "head=$head_sha"
echo "dirty_lines=$dirty_lines"
echo "status_hash=$status_hash"
echo "bundle_hash=$bundle_hash"
echo "hostname=$(hostname 2>/dev/null || echo unknown)"
echo "arch=$(uname -m)"
echo "kernel=$(uname -sr)"
echo "rustc=$(rustc --version 2>/dev/null || echo missing)"
echo "cargo=$(cargo --version 2>/dev/null || echo missing)"
echo "utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "fingerprint=$fingerprint"
