#!/usr/bin/env bash
# P1.3a — SSE progressive stream e2e (chunked event-stream, no full-buffer claim).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

echo "P13A_SSE_STREAM: running cargo test -p exyonq-mod-proxy --test p13a_sse_progressive"
cargo test -p exyonq-mod-proxy --test p13a_sse_progressive -- --nocapture

echo "P13A_SSE_STREAM: SSE_REAL_E2E=PASS (progressive event-stream read)"
exit 0
