#!/usr/bin/env bash
# PS3A D1 dependency gate: platform-linux → core (+ optional mechanism module-api);
# reverse edges forbidden.
#
# Usage:
#   bash scripts/architecture/verify-ps3a-platform-linux-d1.sh
#   bash scripts/architecture/verify-ps3a-platform-linux-d1.sh --selftest
set -euo pipefail

_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_ROOT="$(cd "$_DIR/../.." && pwd)"
_SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"

ROOT="${EXYONQ_PS3A_ROOT:-$_ROOT}"
cd "$ROOT"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify-ps3a-platform-linux-d1: ripgrep (rg) required on PATH" >&2
  exit 2
fi

fail() {
  echo "verify-ps3a-platform-linux-d1: FAIL: $*" >&2
  exit 1
}

ok() {
  echo "verify-ps3a-platform-linux-d1: $*"
}

# Print [dependencies] section only (exclude [dev-dependencies] / other tables).
prod_deps_section() {
  local manifest="$1"
  awk '
    BEGIN { in_deps=0 }
    /^\[/ {
      if ($0 ~ /^\[dependencies\]/) { in_deps=1; next }
      in_deps=0
    }
    in_deps { print }
  ' "$manifest"
}

check_forward_dep() {
  local manifest="crates/exyonq-platform-linux/Cargo.toml"
  [[ -f "$manifest" ]] || fail "missing $manifest"
  local prod
  prod="$(prod_deps_section "$manifest")"
  if ! printf '%s\n' "$prod" | grep -qE 'exyonq-core'; then
    fail "platform-linux [dependencies] must include exyonq-core"
  fi
  # PS3A-PM3-F2: exyonq-module-api allowed (mechanism static_epoll/static_wire only).
  # Still forbid runtime-plan and concrete module crates.
  if printf '%s\n' "$prod" | grep -qE 'exyonq-runtime-plan|exyonq-mod-|path = "\.\./\.\./modules"'; then
    fail "platform-linux [dependencies] must not depend on runtime-plan or exyonq-mod-*"
  fi
  if ! printf '%s\n' "$prod" | grep -qE 'exyonq-module-api|path = "\.\./\.\./module-api"'; then
    fail "platform-linux [dependencies] must include exyonq-module-api (F2 mechanism edge)"
  fi
  ok "platform → core + module-api (mechanism) present (prod graph)"
}

check_module_api_allowlist() {
  # Only static_epoll / static_wire may be imported from module-api in platform sources.
  local hits
  hits="$(rg -n --type rust 'exyonq_module_api::' crates/exyonq-platform-linux/src 2>/dev/null || true)"
  if [[ -n "$hits" ]]; then
    local bad
    bad="$(printf '%s\n' "$hits" | rg -v 'exyonq_module_api::(static_epoll|static_wire)(::|$|\s|;)' || true)"
    if [[ -n "$bad" ]]; then
      echo "$bad" >&2
      fail "platform may only import exyonq_module_api::{static_epoll,static_wire}"
    fi
  fi
  # Also catch `use exyonq_module_api::{foo}` braced imports excluding allowlist.
  local braced
  braced="$(rg -n --type rust 'use exyonq_module_api::\{' crates/exyonq-platform-linux/src 2>/dev/null || true)"
  if [[ -n "$braced" ]]; then
    while IFS= read -r line; do
      if ! printf '%s\n' "$line" | rg -q 'static_epoll|static_wire'; then
        echo "$line" >&2
        fail "braced module-api import must be static_epoll/static_wire only"
      fi
      if printf '%s\n' "$line" | rg -q 'fcgi_|htaccess|proxy_dispatch|route_rules|kernel_control|tls_runtime|discovery|acme_|cache::|observability|http3_|reload_runtime|cross_cutting|static_dispatch|static_paths|proxy_wire|kernel_observation'; then
        echo "$line" >&2
        fail "module-api import includes non-allowlisted policy/registry surface"
      fi
    done <<<"$braced"
  fi
  ok "module-api imports allowlisted (static_epoll/static_wire only)"
}

check_no_reverse_cargo() {
  local f
  for f in \
    core/Cargo.toml \
    module-api/Cargo.toml \
    crates/exyonq-runtime-plan/Cargo.toml \
    crates/exyonq-module-pipeline/Cargo.toml; do
    [[ -f "$f" ]] || continue
    if grep -qE 'exyonq-platform-linux' "$f"; then
      fail "forbidden Cargo dependency on exyonq-platform-linux in $f"
    fi
  done

  local manifest
  for manifest in modules/*/Cargo.toml crates/exyonq-mod-*/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    if grep -qE 'exyonq-platform-linux' "$manifest"; then
      fail "forbidden Cargo dependency on exyonq-platform-linux in $manifest"
    fi
  done
  ok "no reverse Cargo edges (core/mods/runtime-plan/module-api → platform-linux)"
}

check_no_reverse_rust() {
  local matches
  matches="$(rg -n --type rust 'exyonq_platform_linux' \
    core/ \
    module-api/ \
    crates/exyonq-runtime-plan/ \
    crates/exyonq-module-pipeline/ \
    modules/ \
    crates/exyonq-mod-*/ \
    2>/dev/null || true)"
  if [[ -n "$matches" ]]; then
    echo "$matches" >&2
    fail "forbidden Rust import of exyonq_platform_linux outside composition root"
  fi
  ok "no reverse Rust imports of exyonq_platform_linux"
}

check_no_worker_duplication() {
  # PS3A-PM1/PM2/PM3-R2: sync_accept + io_uring + epoll live under platform-linux only.
  # wire_dispatch must not be under platform yet (PM4/PS3B pending).
  local dup
  dup="$(find crates/exyonq-platform-linux -type f -name 'wire_dispatch.rs' 2>/dev/null || true)"
  if [[ -n "$dup" ]]; then
    echo "$dup" >&2
    fail "wire_dispatch must not be under platform-linux before authorized move"
  fi
  [[ -f crates/exyonq-platform-linux/src/sync_accept.rs ]] \
    || fail "PM1: expected crates/exyonq-platform-linux/src/sync_accept.rs"
  [[ -f crates/exyonq-platform-linux/src/conn_pool.rs ]] \
    || fail "PM1: expected crates/exyonq-platform-linux/src/conn_pool.rs (ConnectionPool)"
  [[ -f crates/exyonq-platform-linux/src/io_uring_worker.rs ]] \
    || fail "PM2: expected crates/exyonq-platform-linux/src/io_uring_worker.rs"
  [[ -f crates/exyonq-platform-linux/src/epoll_worker.rs ]] \
    || fail "PM3-R2: expected crates/exyonq-platform-linux/src/epoll_worker.rs"
  if [[ -f core/src/server/sync_accept.rs ]]; then
    fail "PM1: core/src/server/sync_accept.rs must be removed (single implementation in platform)"
  fi
  if [[ -f core/src/server/io_uring_worker.rs ]]; then
    fail "PM2: core/src/server/io_uring_worker.rs must be removed (single implementation in platform)"
  fi
  if [[ -f core/src/server/epoll_worker.rs ]]; then
    fail "PM3-R2: core/src/server/epoll_worker.rs must be removed (single implementation in platform)"
  fi
  [[ -f core/src/server/epoll_start.rs ]] \
    || fail "PM3-R2: expected core/src/server/epoll_start.rs (composition + hook shim)"
  if ! rg -q 'struct SyncBenchCache' core/src/server/conn_pool.rs; then
    fail "SyncBenchCache must remain in core/src/server/conn_pool.rs"
  fi
  if rg -n --type rust -e 'use .*SyncBenchCache' -e '::SyncBenchCache' \
      crates/exyonq-platform-linux/src >/dev/null 2>&1; then
    fail "platform-linux must not import/expose SyncBenchCache"
  fi
  ok "PM1/PM2/PM3-R2 worker ownership; epoll single impl in platform"
}

check_platform_crate_surface() {
  local lib="crates/exyonq-platform-linux/src/lib.rs"
  [[ -f "$lib" ]] || fail "missing $lib"
  if ! rg -q 'INTERNAL WORKSPACE PLATFORM CRATE' "$lib"; then
    fail "lib.rs must document INTERNAL WORKSPACE PLATFORM CRATE"
  fi
  if ! rg -q 'NOT STABLE PUBLIC API' "$lib"; then
    fail "lib.rs must document NOT STABLE PUBLIC API"
  fi
  if rg -n --type rust \
      -e 'use .*::(Shared)?ServerState' \
      -e 'use .*ProxyClient' \
      -e '::ProxyClient' \
      -e 'CoreConnectionExecutor' \
      -e 'use .*SyncBenchCache' \
      -e '::SyncBenchCache' \
      crates/exyonq-platform-linux/src >/dev/null 2>&1; then
    fail "platform must not import SharedServerState / ProxyClient / executor / SyncBenchCache"
  fi
  local bad_server
  bad_server="$(rg -n --type rust 'exyonq_core::server::' crates/exyonq-platform-linux/src \
    | rg -v 'OsWorkerGuard|register_sync_accept_start|register_io_uring_start|register_epoll_|sync_accept_env_enabled|io_uring_env_enabled|epoll_static_env_enabled|epoll_listen_env_enabled' \
    || true)"
  if [[ -n "$bad_server" ]]; then
    echo "$bad_server" >&2
    fail "platform may only use server composition seam (OsWorkerGuard / register_*_start)"
  fi
  if ! rg -q 'register_io_uring_start|start_io_uring_workers' crates/exyonq-platform-linux/src; then
    fail "platform crate must register io_uring starter (PM2)"
  fi
  if ! rg -q 'register_epoll_listen_start|start_epoll_listen_workers' crates/exyonq-platform-linux/src; then
    fail "platform crate must register epoll listen starter (PM3-R2)"
  fi
  if ! rg -q 'attach_epoll_connection' crates/exyonq-platform-linux/src/epoll_worker.rs; then
    fail "platform epoll must use F1 attach_epoll_connection"
  fi
  if ! rg -q 'attach_epoll_keepalive_transfer' crates/exyonq-platform-linux/src/epoll_worker.rs; then
    fail "platform epoll must use F2 attach_epoll_keepalive_transfer"
  fi
  if rg -n --type rust \
      -e 'ConnectionLifecycleToken' \
      -e 'try_enter' \
      -e 'spawn_hyper_admitted' \
      -e 'admit_epoll_accept' \
      crates/exyonq-platform-linux/src >/dev/null 2>&1; then
    fail "platform must not use lifecycle token / try_enter / executor admit/hyper APIs"
  fi
  if ! rg -q 'PlatformConnectionEntry' "$lib"; then
    fail "platform crate must consume PlatformConnectionEntry facade"
  fi
  if ! rg -q 'serve_via_entry|serve_accepted_connection' "$lib"; then
    fail "platform crate must demonstrate facade serve entry"
  fi
  if ! rg -q 'PlatformConnectionAdmission|plan_wire_decision|HyperHandoff' "$lib"; then
    fail "platform crate must demonstrate PS1C contract consumption"
  fi
  if ! rg -q 'EpollKeepaliveTransfer|attach_epoll_keepalive_transfer|enqueue_keepalive_transfer' crates/exyonq-platform-linux/src; then
    fail "platform crate must demonstrate F2 keepalive transfer consumption surface"
  fi
  if ! rg -q 'static_epoll|static_wire' crates/exyonq-platform-linux/src; then
    fail "platform crate must demonstrate allowlisted module-api mechanism imports (F2)"
  fi
  ok "platform crate documents INTERNAL API and consumes PS1C/F1/F2 facade + mechanism seam"
}

run_checks() {
  echo "verify-ps3a-platform-linux-d1: scanning $ROOT ..."
  check_forward_dep
  check_module_api_allowlist
  check_no_reverse_cargo
  check_no_reverse_rust
  check_no_worker_duplication
  check_platform_crate_surface
  ok "PASS (D1)"
}

run_selftest() {
  echo "verify-ps3a-platform-linux-d1: --selftest ..."
  local tmp rc
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" RETURN

  mkdir -p "$tmp/core" "$tmp/crates/exyonq-platform-linux/src" "$tmp/modules/probe"
  printf '%s\n' '[package]
name = "exyonq-core"
version = "0.0.0"
[dependencies]
exyonq-platform-linux = { path = "../crates/exyonq-platform-linux" }
' >"$tmp/core/Cargo.toml"
  printf '%s\n' '[package]
name = "exyonq-platform-linux"
version = "0.0.0"
[dependencies]
exyonq-core = { path = "../../core" }
exyonq-module-api = { path = "../../module-api" }
' >"$tmp/crates/exyonq-platform-linux/Cargo.toml"
  printf '%s\n' '//! INTERNAL WORKSPACE PLATFORM CRATE
//! NOT STABLE PUBLIC API
use exyonq_core::kernel::{
    PlatformConnectionAdmission, PlatformConnectionEntry, plan_wire_decision, HyperHandoff,
    EpollKeepaliveTransfer,
};
use exyonq_module_api::static_epoll;
use exyonq_module_api::static_wire;
pub fn serve_via_entry(entry: &PlatformConnectionEntry) { let _ = entry; }
pub fn f2(entry: &PlatformConnectionEntry, t: EpollKeepaliveTransfer) {
  let _ = entry.attach_epoll_keepalive_transfer(t);
  let _ = static_epoll::interest_reading;
  let _ = static_wire::p1_bench_wire_rodata;
}
' >"$tmp/crates/exyonq-platform-linux/src/lib.rs"
  mkdir -p "$tmp/core/src/server" "$tmp/crates/exyonq-platform-linux/src" "$tmp/module-api"
  # Negative tree keeps obsolete core workers so duplication checks also fail if reached.
  touch "$tmp/core/src/server/epoll_worker.rs" \
        "$tmp/core/src/server/io_uring_worker.rs" \
        "$tmp/core/src/server/conn_pool.rs"
  printf '%s\n' 'struct SyncBenchCache;' >"$tmp/core/src/server/conn_pool.rs"
  printf '%s\n' '// sync_accept' >"$tmp/crates/exyonq-platform-linux/src/sync_accept.rs"
  printf '%s\n' '// conn_pool' >"$tmp/crates/exyonq-platform-linux/src/conn_pool.rs"
  printf '%s\n' '// io_uring_worker' >"$tmp/crates/exyonq-platform-linux/src/io_uring_worker.rs"
  printf '%s\n' '// epoll_worker' >"$tmp/crates/exyonq-platform-linux/src/epoll_worker.rs"
  printf '%s\n' '[package]
name = "exyonq-module-api"
version = "0.0.0"
' >"$tmp/module-api/Cargo.toml"

  rc=0
  EXYONQ_PS3A_ROOT="$tmp" bash "$_SCRIPT" >/dev/null 2>&1 || rc=$?
  if [[ "$rc" -eq 0 ]]; then
    echo "verify-ps3a-platform-linux-d1: --selftest FAILED: expected fail on core→platform Cargo edge" >&2
    return 1
  fi

  rc=0
  bash "$_SCRIPT" >/dev/null 2>&1 || rc=$?
  if [[ "$rc" -ne 0 ]]; then
    echo "verify-ps3a-platform-linux-d1: --selftest FAILED: clean repo must PASS (got $rc)" >&2
    return 1
  fi
  echo "verify-ps3a-platform-linux-d1: --selftest OK"
  return 0
}

case "${1:-}" in
  -h | --help)
    sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  --selftest)
    run_selftest
    exit $?
    ;;
  "")
    run_checks
    ;;
  *)
    echo "unknown arg: $1" >&2
    exit 2
    ;;
esac
