#!/usr/bin/env bash
# KD2.2 CI slice — fmt/clippy/tests/guards for static dispatch shell.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

echo "=== KD2.2 fmt (module-api, mod-static, core execute_backend) ==="
cargo fmt --all -- --check

echo "=== KD2.2 clippy (mod-static + core) ==="
cargo clippy -p exyonq-mod-static -p exyonq-core -- -D warnings

echo "=== KD2.2 unit tests ==="
cargo test -p exyonq-mod-static
cargo test -p exyonq-module-api static_dispatch
cargo test -p exyonq-core execute_backend::tests
cargo test -p exyonq-core --test kd2_static_dispatch_test
cargo test -p exyonq-core --test kd1_fcgi_observability_test

echo "=== KD2.2 guards ==="
bash scripts/architecture/verify-kd2-static-guards.sh baseline
bash scripts/architecture/verify-kd1-ci.sh

echo "verify-kd2-ci: OK"
