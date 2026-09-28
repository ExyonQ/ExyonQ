#!/usr/bin/env bash
# KD3.4 — controlled Plan12 cache concurrency diagnostic (Netcup/remote).
# Does NOT re-run full native gate. Stops on first failure; no auto-retry for PASS.
set -euo pipefail

WORKSPACE="${1:-/root/exyonq-dev-soak-src}"
EXPECTED_FP="${2:-010481be9ee40b8f74a6c720cc0560ad9d86d01a3a14f5e59bf9f30631d1b452}"
OUT_DIR="${3:-}"
ITERATIONS="${KD34_DIAG_ITERATIONS:-10}"

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$(pwd)/benchmarks/results-dev/kd34-plan12-diagnostic-$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$OUT_DIR"

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:/root/.cargo/bin:${PATH:-/usr/bin:/bin}"
export RUST_BACKTRACE=1

log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$OUT_DIR/diagnostic.log"; }

log "=== KD3.4 Plan12 diagnostic ==="
log "workspace=$WORKSPACE"
log "expected_fingerprint=$EXPECTED_FP"
log "hostname=$(hostname) arch=$(uname -m) kernel=$(uname -sr)"
log "rustc=$(rustc --version 2>/dev/null || echo missing)"
log "cargo=$(cargo --version 2>/dev/null || echo missing)"
log "load=$(uptime 2>/dev/null || true)"

FP_SCRIPT="scripts/remote/kd3-tree-fingerprint.sh"
if [[ -f "$FP_SCRIPT" ]]; then
  bash "$FP_SCRIPT" "$WORKSPACE" | tee "$OUT_DIR/fingerprint.txt"
  ACTUAL_FP="$(awk -F= '/^fingerprint=/{print $2}' "$OUT_DIR/fingerprint.txt")"
  if [[ "$ACTUAL_FP" != "$EXPECTED_FP" ]]; then
    log "ERROR: fingerprint mismatch expected=$EXPECTED_FP actual=$ACTUAL_FP"
    exit 2
  fi
  log "fingerprint: OK"
else
  log "WARN: fingerprint script missing"
fi

log "--- environment ---"
env | LC_ALL=C sort | tee "$OUT_DIR/env.txt" >/dev/null
log "RUST_BACKTRACE=${RUST_BACKTRACE:-unset}"
log "CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-default}"

log "--- residual processes (exyonq/cargo test) ---"
ps aux | rg -i 'exyonq|cargo test|plan12_cache' | tee "$OUT_DIR/ps-residual.txt" || true

log "--- listening ports (sample) ---"
ss -ltnp 2>/dev/null | head -40 | tee "$OUT_DIR/ss-listen.txt" || true

log "--- build test binary ---"
cargo test -p exyonq-core --test plan12_cache_concurrency_test --no-run 2>&1 | tee "$OUT_DIR/build-test-bin.log"
BIN="$(ls -1 target/debug/deps/plan12_cache_concurrency_test-*.exe target/debug/deps/plan12_cache_concurrency_test-* 2>/dev/null | head -1 || true)"
if [[ -z "$BIN" ]]; then
  BIN="$(find target/debug/deps -maxdepth 1 -name 'plan12_cache_concurrency_test-*' -perm -111 2>/dev/null | head -1)"
fi
log "test_binary=$BIN"
[[ -n "$BIN" && -x "$BIN" ]] || {
  log "ERROR: test binary not found"
  exit 2
}

run_case() {
  local label="$1"
  shift
  local logfile="$OUT_DIR/${label}.log"
  log "RUN $label: $*"
  set +e
  "$@" >"$logfile" 2>&1
  local rc=$?
  set -e
  echo "exit_code=$rc" >>"$logfile"
  if [[ $rc -ne 0 ]]; then
    log "FAIL $label exit=$rc (see $logfile)"
    tail -30 "$logfile" | tee -a "$OUT_DIR/diagnostic.log"
    echo "$label:FAIL:$rc" >>"$OUT_DIR/summary.txt"
    return 1
  fi
  log "PASS $label"
  echo "$label:PASS:0" >>"$OUT_DIR/summary.txt"
  return 0
}

# Gate-equivalent: full integration test binary via cargo, serial lib threads
if ! run_case "gate_equiv_cargo_test_threads_1" \
  cargo test -p exyonq-core --test plan12_cache_concurrency_test -- --nocapture --test-threads=1; then
  log "STOP: gate-equivalent command failed"
  exit 1
fi

# Isolated failing test only
if ! run_case "isolated_singleflight_threads_1" \
  cargo test -p exyonq-core --test plan12_cache_concurrency_test singleflight_deduplicates_concurrent_misses -- --nocapture --test-threads=1; then
  log "STOP: isolated serial failed"
  exit 1
fi

if ! run_case "isolated_singleflight_threads_8" \
  cargo test -p exyonq-core --test plan12_cache_concurrency_test singleflight_deduplicates_concurrent_misses -- --nocapture --test-threads=8; then
  log "STOP: isolated parallel test-threads=8 failed"
  exit 1
fi

# Repeat gate-equivalent isolated test
for i in $(seq 1 "$ITERATIONS"); do
  if ! run_case "repeat_${i}_singleflight_threads_1" \
    cargo test -p exyonq-core --test plan12_cache_concurrency_test singleflight_deduplicates_concurrent_misses -- --nocapture --test-threads=1; then
    log "STOP: repeat iteration $i failed"
    exit 1
  fi
done

log "=== ALL DIAGNOSTIC CASES PASS ($ITERATIONS repeats) ==="
echo "verdict=NOT_REPRODUCED_IN_DIAGNOSTIC" >>"$OUT_DIR/summary.txt"
echo "out_dir=$OUT_DIR"
