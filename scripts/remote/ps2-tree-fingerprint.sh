#!/usr/bin/env bash
# PS2 tree fingerprint — binds baseline to exact dirty tree (no commit).
set -euo pipefail

WORKSPACE="${1:-.}"
cd "$WORKSPACE"

git_ok=0
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  git_ok=1
fi

if [[ "$git_ok" == "1" ]]; then
  head_sha="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
  branch="$(git branch --show-current 2>/dev/null || echo detached)"
  dirty_lines="$(git status --short 2>/dev/null | wc -l | tr -d ' ')"
  status_hash="$(
    git status --short 2>/dev/null | LC_ALL=C sort | sha256sum | awk '{print $1}'
  )"
else
  head_sha="rsync-no-git"
  branch="rsync-no-git"
  dirty_lines="0"
  status_hash="e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
  if [[ -f .ps2-sync-manifest ]]; then
    # shellcheck disable=SC1091
    source .ps2-sync-manifest
    head_sha="${PS2_HEAD:-$head_sha}"
    dirty_lines="${PS2_DIRTY_LINES:-$dirty_lines}"
    status_hash="${PS2_STATUS_HASH:-$status_hash}"
  fi
fi
cargo_lock_sha="$(sha256sum Cargo.lock 2>/dev/null | awk '{print $1}' || echo missing)"
cargo_toml_sha="$(sha256sum Cargo.toml 2>/dev/null | awk '{print $1}' || echo missing)"

bundle_hash="$(
  find core/src core/tests module-api/src crates/exyonq-mod-static crates/exyonq-mod-proxy \
    crates/exyonq-mod-fastcgi crates/exyonq-mod-tls crates/exyonq-mod-http3 \
    crates/exyonq-kernel-contract-consumer crates/exyonq-runtime-plan \
    benchmarks/configs benchmarks/docker benchmarks/scenarios \
    docs/benchmarks/platform-split/ps2-collect-fingerprint.sh \
    docs/benchmarks/platform-split/ps2-run-baseline.sh \
    docs/benchmarks/platform-split/ps2-summarize-stats.py \
    docs/benchmarks/platform-split/PS2_HOST_POLICY.md \
    scripts/remote/ps2-tree-fingerprint.sh \
    scripts/remote/validate-ps2-baseline.sh scripts/remote/run-ps2-baseline-freeze.sh \
    scripts/remote/ps2-generate-source-manifest.sh scripts/remote/ps2-verify-remote-identity.sh \
    -type f 2>/dev/null | LC_ALL=C sort | xargs -r sha256sum 2>/dev/null | sha256sum | awk '{print $1}'
)"
# macOS xargs lacks -r; retry without it when bundle_hash empty.
if [[ -z "${bundle_hash:-}" ]]; then
  bundle_hash="$(
    find core/src core/tests module-api/src crates/exyonq-mod-static crates/exyonq-mod-proxy \
      crates/exyonq-mod-fastcgi crates/exyonq-mod-tls crates/exyonq-mod-http3 \
      crates/exyonq-kernel-contract-consumer crates/exyonq-runtime-plan \
      benchmarks/configs benchmarks/docker benchmarks/scenarios \
      docs/benchmarks/platform-split/ps2-collect-fingerprint.sh \
      docs/benchmarks/platform-split/ps2-run-baseline.sh \
      docs/benchmarks/platform-split/ps2-summarize-stats.py \
      docs/benchmarks/platform-split/PS2_HOST_POLICY.md \
      scripts/remote/ps2-tree-fingerprint.sh \
      scripts/remote/validate-ps2-baseline.sh scripts/remote/run-ps2-baseline-freeze.sh \
      scripts/remote/ps2-generate-source-manifest.sh scripts/remote/ps2-verify-remote-identity.sh \
      -type f 2>/dev/null | LC_ALL=C sort | xargs sha256sum 2>/dev/null | sha256sum | awk '{print $1}'
  )"
fi
[[ -n "${bundle_hash:-}" ]] || {
  echo "ERROR: bundle_hash empty" >&2
  exit 2
}

fingerprint="$(printf '%s\n%s\n%s\n%s\n%s' "$head_sha" "$dirty_lines" "$status_hash" "$bundle_hash" "$cargo_lock_sha" | sha256sum | awk '{print $1}')"

# Remote workspaces rsync without .git — bundle-only fingerprint for post-sync verify.
bundle_only_fingerprint="$(printf '%s\n%s' "$bundle_hash" "$cargo_lock_sha" | sha256sum | awk '{print $1}')"

echo "phase=PS2_PERFORMANCE_BASELINE_FREEZE"
echo "branch=$branch"
echo "head=$head_sha"
echo "dirty_lines=$dirty_lines"
echo "status_hash=$status_hash"
echo "bundle_hash=$bundle_hash"
echo "cargo_lock_sha256=$cargo_lock_sha"
echo "cargo_toml_sha256=$cargo_toml_sha"
echo "hostname=$(hostname 2>/dev/null || echo unknown)"
echo "arch=$(uname -m)"
echo "kernel=$(uname -sr)"
echo "rustc=$(rustc --version 2>/dev/null || echo missing)"
echo "cargo=$(cargo --version 2>/dev/null || echo missing)"
echo "docker=$(docker version --format '{{.Server.Version}}' 2>/dev/null || echo missing)"
echo "utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "bundle_only_fingerprint=$bundle_only_fingerprint"
echo "fingerprint=$fingerprint"
