#!/usr/bin/env bash
# Deterministic KD4.14 tree fingerprint (local or remote workspace).
set -euo pipefail

if command -v sha256sum >/dev/null 2>&1; then
  SHA256() { sha256sum | awk '{print $1}'; }
else
  SHA256() { shasum -a 256 | awk '{print $1}'; }
fi

WORKSPACE="${1:-.}"
cd "$WORKSPACE"

MANIFEST="${KD414_SYNC_MANIFEST:-.kd414-sync-manifest}"
if [[ -f "$MANIFEST" ]]; then
  # shellcheck disable=SC1090
  source "$MANIFEST"
  head_sha="${KD414_HEAD:-unknown}"
  dirty_lines="${KD414_DIRTY_LINES:-0}"
  status_hash="${KD414_STATUS_HASH:-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855}"
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
    module-api/src/reload_runtime.rs \
    module-api/src/lib.rs \
    crates/exyonq-reload-runtime/src \
    crates/exyonq-reload-runtime/Cargo.toml \
    core/src/reload/mod.rs \
    core/src/kernel_control_port.rs \
    core/src/lifecycle.rs \
    core/src/server/mod.rs \
    core/Cargo.toml \
    cli/exyonq/src/main.rs \
    cli/exyonq/Cargo.toml \
    tests/integration/bootstrap.rs \
    tests/integration/Cargo.toml \
    core/tests/reload_lifecycle.rs \
    scripts/architecture/verify-kd4-core-residual-guards.sh \
    scripts/remote/kd414-tree-fingerprint.sh \
    scripts/remote/validate-kd414-native.sh \
    scripts/remote/run-kd414-native-gate.sh \
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
