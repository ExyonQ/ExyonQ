#!/usr/bin/env bash
# P14V042 — source revision contract (no long OCI build; structural + resolver gates).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }

# .git must remain excluded from OCI context (do not reverse this as a "fix").
grep -qE '^\.git$' .dockerignore || fail ".dockerignore must list .git (exact line)"
pass ".dockerignore excludes .git"

DF=packaging/docker/Dockerfile
grep -q 'EXYONQ_SOURCE_REVISION' "$DF" || fail "Dockerfile must set EXYONQ_SOURCE_REVISION for cargo"
grep -q 'EXYONQ_OFFICIAL_RELEASE' "$DF" || fail "Dockerfile must gate EXYONQ_OFFICIAL_RELEASE"
grep -q 'org.opencontainers.image.revision' "$DF" || fail "Dockerfile must label revision"
grep -q 'EXYONQ_GIT_REVISION' "$DF" || fail "Dockerfile must accept EXYONQ_GIT_REVISION build-arg"
# Must not COPY .git
if grep -nE 'COPY[[:space:]]+\.git|ADD[[:space:]]+\.git' "$DF"; then
  fail "Dockerfile must not COPY/ADD .git"
fi
pass "Dockerfile injects revision without copying .git"

# WS6 packaging must export revision before cargo build
grep -q 'EXYONQ_SOURCE_REVISION=' scripts/release/p15-ws6-build-artifacts.sh \
  || fail "p15-ws6-build-artifacts.sh must export EXYONQ_SOURCE_REVISION"
grep -q 'EXYONQ_OFFICIAL_RELEASE' scripts/release/p15-ws6-build-artifacts.sh \
  || fail "p15-ws6-build-artifacts.sh must set EXYONQ_OFFICIAL_RELEASE"
pass "WS6 build-artifacts exports source revision"

# CI release docker job must pass official build-args
grep -q 'EXYONQ_GIT_REVISION=\${{ github.sha }}' .github/workflows/release.yml \
  || fail "release.yml docker must pass github.sha as EXYONQ_GIT_REVISION"
grep -q 'EXYONQ_OFFICIAL_RELEASE=1' .github/workflows/release.yml \
  || fail "release.yml docker must set EXYONQ_OFFICIAL_RELEASE=1"
pass "release.yml official OCI build-args present"

# Resolver unit tests (binary/OCI coherence is exercised on Netcup/Oracle smoke)
cargo test -p exyonq --test source_revision_resolve -- --nocapture
cargo test -p exyonqctl --test source_revision_resolve -- --nocapture
pass "source_revision_resolve tests"

echo "VERDICT=PASS"
echo "P14V042_SOURCE_REVISION_CONTRACT=PASS"
