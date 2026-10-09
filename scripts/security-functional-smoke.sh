#!/usr/bin/env bash
# Quick security-related functional checks (no full docker stack).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=== Security functional smoke ==="

cargo run -q -p exyonq -- validate -c tests/fixtures/tls-minimal.toml
cargo run -q -p exyonq -- validate -c tests/fixtures/http2-http3.toml

cargo test -p exyonq-core static_files::tests::blocks_path_traversal

echo "Security functional smoke: OK"
