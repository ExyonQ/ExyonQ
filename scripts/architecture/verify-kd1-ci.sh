#!/usr/bin/env bash
# KD1.1 — CI slice for FastCGI kernel extraction (excludes C1 xtask debt).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

KD1_PACKAGES=(exyonq-module-api exyonq-core exyonq-mod-fastcgi exyonq)

echo "=== KD1.1 fmt check (packages) ==="
for pkg in "${KD1_PACKAGES[@]}"; do
  cargo fmt -p "$pkg" -- --check
done

echo "=== KD1.1 clippy ==="
cargo clippy -p exyonq-module-api -p exyonq-core -p exyonq-mod-fastcgi -p exyonq --all-targets -- -D warnings

echo "=== KD1.1 tests ==="
cargo test -p exyonq-mod-fastcgi
cargo test -p exyonq-core --test kd1_fcgi_observability_test --test plan08_pr1_test \
  --test plan08_pr2_test --test plan08_pr3_test --test plan08_pr5b_test \
  --test plan08_pr5b2_test --test plan12_fastcgi_cache_test \
  --test plan12_fastcgi_cache_concurrency_test -- --test-threads=1

echo "=== KD1.1 guards ==="
bash scripts/architecture/verify-kd0-fcgi-guards.sh

echo "verify-kd1-ci: OK"
