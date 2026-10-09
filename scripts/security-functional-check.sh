#!/usr/bin/env bash
# Quick security-related functional checks (no full docker stack).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=== Security functional check ==="

cargo run -q -p exyonq -- validate -c tests/fixtures/tls-minimal.toml
cargo run -q -p exyonq -- validate -c tests/fixtures/http2-http3.toml

# Run the concrete integration target, not a stale filter that can report
# success after executing zero tests.
cargo test -p exyonq-core --test kd2_static_dispatch_test

echo "Security functional check: OK"
