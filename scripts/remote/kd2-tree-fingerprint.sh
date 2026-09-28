#!/usr/bin/env bash
# Deterministic KD2 tree fingerprint (local or remote workspace).
# Output: key=value lines + fingerprint=<sha256> on last line.
set -euo pipefail

WORKSPACE="${1:-.}"
cd "$WORKSPACE"

head_sha="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
dirty_lines="$(git status --short 2>/dev/null | wc -l | tr -d ' ')"
if [[ -z "$dirty_lines" ]]; then
  dirty_lines=0
fi

status_hash="$(
  git status --short 2>/dev/null | LC_ALL=C sort | sha256sum | awk '{print $1}'
)"
if [[ -z "$status_hash" ]]; then
  status_hash=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
fi

bundle_hash="$(
  find core/src module-api/src crates/exyonq-mod-static cli/exyonq \
    scripts/architecture/verify-kd2-static-guards.sh \
    scripts/e2e/suites/static-file-suite.sh \
    scripts/remote/validate-kd2-native.sh \
    scripts/remote/kd2-tree-fingerprint.sh \
    -type f 2>/dev/null | LC_ALL=C sort | xargs sha256sum 2>/dev/null | sha256sum | awk '{print $1}'
)"

fingerprint="$(printf '%s\n%s\n%s\n%s' "$head_sha" "$dirty_lines" "$status_hash" "$bundle_hash" | sha256sum | awk '{print $1}')"

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
