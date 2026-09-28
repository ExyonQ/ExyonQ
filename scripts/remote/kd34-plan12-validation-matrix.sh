#!/usr/bin/env bash
# KD3.4 — Plan12 deterministic validation matrix (50× isolated + 20× full bin).
# Stops on first failure; restart matrix from zero after any fix.
set -euo pipefail

WORKSPACE="${1:-.}"
ISOLATED_RUNS="${KD34_ISOLATED_RUNS:-50}"
FULL_BIN_RUNS="${KD34_FULL_BIN_RUNS:-20}"
OUT_DIR="${2:-}"

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$(pwd)/benchmarks/results-dev/kd34-plan12-matrix-$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$OUT_DIR"

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:/root/.cargo/bin:${PATH:-/usr/bin:/bin}"
export RUST_BACKTRACE=1

log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$OUT_DIR/matrix.log"; }

log "=== KD3.4 Plan12 validation matrix ==="
log "workspace=$WORKSPACE isolated=$ISOLATED_RUNS full_bin=$FULL_BIN_RUNS"
log "hostname=$(hostname) arch=$(uname -m)"

log "--- build test binary ---"
cargo test -p exyonq-core --test plan12_cache_concurrency_test --no-run 2>&1 | tee "$OUT_DIR/build.log"

BIN="$(find target/debug/deps -maxdepth 1 -name 'plan12_cache_concurrency_test-*' -perm -111 2>/dev/null | head -1)"
[[ -n "$BIN" && -x "$BIN" ]] || {
  log "ERROR: test binary not found"
  exit 2
}
log "test_binary=$BIN"

run_isolated() {
  local i="$1"
  local logfile="$OUT_DIR/isolated_${i}.log"
  set +e
  cargo test -p exyonq-core --test plan12_cache_concurrency_test \
    singleflight_deduplicates_concurrent_misses \
    -- --nocapture --test-threads=1 >"$logfile" 2>&1
  local rc=$?
  set -e
  if [[ $rc -ne 0 ]]; then
    log "FAIL isolated iteration $i exit=$rc (see $logfile)"
    tail -40 "$logfile" | tee -a "$OUT_DIR/matrix.log"
    echo "isolated:$i:FAIL:$rc" >>"$OUT_DIR/summary.txt"
    exit 1
  fi
  echo "isolated:$i:PASS" >>"$OUT_DIR/summary.txt"
}

run_full_bin() {
  local i="$1"
  local logfile="$OUT_DIR/fullbin_${i}.log"
  set +e
  cargo test -p exyonq-core --test plan12_cache_concurrency_test \
    -- --nocapture --test-threads=1 >"$logfile" 2>&1
  local rc=$?
  set -e
  if [[ $rc -ne 0 ]]; then
    log "FAIL full bin iteration $i exit=$rc (see $logfile)"
    tail -40 "$logfile" | tee -a "$OUT_DIR/matrix.log"
    echo "fullbin:$i:FAIL:$rc" >>"$OUT_DIR/summary.txt"
    exit 1
  fi
  echo "fullbin:$i:PASS" >>"$OUT_DIR/summary.txt"
}

log "--- phase 1: ${ISOLATED_RUNS}× isolated singleflight ---"
for i in $(seq 1 "$ISOLATED_RUNS"); do
  log "isolated $i/$ISOLATED_RUNS"
  run_isolated "$i"
done

log "--- phase 2: ${FULL_BIN_RUNS}× full plan12 bin ---"
for i in $(seq 1 "$FULL_BIN_RUNS"); do
  log "full bin $i/$FULL_BIN_RUNS"
  run_full_bin "$i"
done

log "=== MATRIX PASS (${ISOLATED_RUNS} isolated + ${FULL_BIN_RUNS} full bin) ==="
echo "verdict=PLAN12_DETERMINISTIC_PASS" >>"$OUT_DIR/summary.txt"
echo "out_dir=$OUT_DIR"
