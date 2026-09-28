#!/usr/bin/env bash
# KD3.4 — isolated WebSocket echo validation (50× consecutive Rust tests, stop on first fail).
set -euo pipefail

WORKSPACE="${1:-.}"
RUNS="${KD34_WS_ISOLATED_RUNS:-50}"
OUT_DIR="${2:-}"

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$(pwd)/benchmarks/results-dev/kd34-ws-matrix-$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$OUT_DIR"

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:/root/.cargo/bin:${PATH:-/usr/bin:/bin}"

log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$OUT_DIR/matrix.log"; }

log "=== KD3.4 WebSocket isolated matrix (Rust deterministic) ==="
log "workspace=$WORKSPACE runs=$RUNS"
log "hostname=$(hostname) arch=$(uname -m)"

log "--- build + compile tests ---"
cargo test -p exyonq-core ws_tunnel_tests --no-run 2>&1 | tee "$OUT_DIR/build-core.log"
cargo test -p exyonq-mod-proxy websocket::tests --no-run 2>&1 | tee "$OUT_DIR/build-mod-proxy.log"

run_once() {
  cargo test -p exyonq-core ws_tunnel_tests -- --test-threads=1 >/dev/null 2>&1 \
    && cargo test -p exyonq-mod-proxy websocket::tests -- --test-threads=1 >/dev/null 2>&1
}

for i in $(seq 1 "$RUNS"); do
  logfile="$OUT_DIR/ws_${i}.log"
  set +e
  run_once >"$logfile" 2>&1
  rc=$?
  set -e
  if [[ $rc -ne 0 ]]; then
    log "FAIL iteration $i exit=$rc"
    tail -40 "$logfile" | tee -a "$OUT_DIR/matrix.log"
    echo "ws:$i:FAIL:$rc" >>"$OUT_DIR/summary.txt"
    exit 1
  fi
  echo "ws:$i:PASS" >>"$OUT_DIR/summary.txt"
  log "PASS iteration $i/$RUNS"
done

log "=== WEBSOCKET_ECHO_ISOLATED = ${RUNS}/${RUNS} PASS ==="
exit 0
