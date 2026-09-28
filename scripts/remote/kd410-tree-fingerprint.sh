#!/usr/bin/env bash
# Deterministic KD4.10 tree fingerprint (local or remote workspace).
set -euo pipefail

if command -v sha256sum >/dev/null 2>&1; then
  SHA256() { sha256sum | awk '{print $1}'; }
else
  SHA256() { shasum -a 256 | awk '{print $1}'; }
fi

WORKSPACE="${1:-.}"
cd "$WORKSPACE"

MANIFEST="${KD410_SYNC_MANIFEST:-.kd410-sync-manifest}"
if [[ -f "$MANIFEST" ]]; then
  # shellcheck disable=SC1090
  source "$MANIFEST"
  head_sha="${KD410_HEAD:-unknown}"
  dirty_lines="${KD410_DIRTY_LINES:-0}"
  status_hash="${KD410_STATUS_HASH:-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855}"
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
    crates/exyonq-runtime-plan/src \
    crates/exyonq-ops-runtime/src \
    crates/exyonq-discovery-runtime/src \
    core/src/discovery_overlay.rs \
    core/src/lifecycle.rs \
    core/src/kernel_control_port.rs \
    core/src/server/mod.rs \
    core/src/reload/mod.rs \
    core/src/server/handler.rs \
    core/src/server/wire_dispatch.rs \
    core/src/execute_backend.rs \
    core/src/lib.rs \
    config/merge/src \
    module-api/src \
    crates/exyonq-mod-proxy/src \
    cli/exyonq/src/main.rs \
    tests/integration/bootstrap.rs \
    tests/integration/security.rs \
    scripts/architecture/verify-kd4-core-residual-guards.sh \
    scripts/architecture/verify-kd3-proxy-guards.sh \
    scripts/architecture/verify-kd2-static-guards.sh \
    scripts/architecture/verify-kd2-ci.sh \
    scripts/architecture/verify-kd1-ci.sh \
    scripts/architecture/verify-kd0-fcgi-guards.sh \
    scripts/remote/kd410-tree-fingerprint.sh \
    scripts/remote/validate-kd410-native.sh \
    scripts/remote/run-kd410-native-gate.sh \
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
