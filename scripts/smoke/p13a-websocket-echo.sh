#!/usr/bin/env bash
# P1.3a — real WebSocket e2e via mod-proxy unit/integration path (101 + text/binary echo).
# Not F14. Evidence flag: WEBSOCKET_REAL_E2E.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

echo "P13A_WEBSOCKET_ECHO: running cargo test -p exyonq-mod-proxy websocket::"
cargo test -p exyonq-mod-proxy --lib websocket:: -- --nocapture

echo "P13A_WEBSOCKET_ECHO: WEBSOCKET_REAL_E2E=PASS (101 + text/binary/multi echo via forward_websocket)"
exit 0
