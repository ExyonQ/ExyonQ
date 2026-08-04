#!/usr/bin/env bash
# R3A local smoke — NOT Linux evidence.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$ROOT/tools/r3-p4-mock/target/release/r3-p4-mock"
PORT="${MOCK_PORT:-19000}"
EV="${R3_EV:-$ROOT/.exyonq-local/tmp/r3-20260802}"
mkdir -p "$EV"

if [[ ! -x "$BIN" ]]; then
  cargo build --release --manifest-path "$ROOT/tools/r3-p4-mock/Cargo.toml"
fi

MOCK_PORT="$PORT" "$BIN" >"$EV/r3a-smoke-mock.log" 2>&1 &
PID=$!
cleanup() { kill "$PID" 2>/dev/null || true; }
trap cleanup EXIT
for i in $(seq 1 50); do curl -fsS "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1 && break; sleep 0.1; done

curl -fsS "http://127.0.0.1:${PORT}/health" | tee "$EV/r3a-health.txt"
BODY="$(curl -fsS "http://127.0.0.1:${PORT}/api/")"
LEN="$(printf '%s' "$BODY" | wc -c | tr -d ' ')"
echo "api_body_len=$LEN" | tee "$EV/r3a-api-len.txt"
test "$LEN" = "1024"
echo "R3A_SMOKE=PASS"
