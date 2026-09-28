#!/usr/bin/env bash
# KD3.4 — full proxy E2E validation (10× consecutive 12/12, stop on first fail).
set -euo pipefail

WORKSPACE="${1:-.}"
RUNS="${KD34_E2E_RUNS:-10}"
OUT_DIR="${2:-}"

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$(pwd)/benchmarks/results-dev/kd34-e2e-matrix-$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$OUT_DIR"

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:/root/.cargo/bin:${PATH:-/usr/bin:/bin}"

log() { echo "[$(date -u +%H:%M:%S)] $*" | tee -a "$OUT_DIR/matrix.log"; }

log "=== KD3.4 full E2E matrix ==="
log "workspace=$WORKSPACE runs=$RUNS"
log "hostname=$(hostname) arch=$(uname -m)"

if [[ "$(uname -s)" != "Linux" ]]; then
  log "ERROR: Linux only"
  exit 2
fi

log "--- build exyonq ---"
cargo build -p exyonq --bin exyonq 2>&1 | tee "$OUT_DIR/build.log"

for i in $(seq 1 "$RUNS"); do
  logfile="$OUT_DIR/e2e_.log"
  set +e
  bash scripts/e2e/suites/proxy-suite.sh >"$logfile" 2>&1
  rc=$?
  set -e
  if [[ $rc -ne 0 ]]; then
    log "FAIL iteration $i exit=$rc"
    tail -60 "$logfile" | tee -a "$OUT_DIR/matrix.log"
    echo "e2e::FAIL:$rc" >>"$OUT_DIR/summary.txt"
    exit 1
  fi
  pass_count="$(grep -o 'PASS=[0-9]*' "$logfile" | tail -1 | cut -d= -f2 || true)"
  fail_count="$(grep -o 'FAIL=[0-9]*' "$logfile" | tail -1 | cut -d= -f2 || true)"
  if [[ "${pass_count:-0}" != "12" || "${fail_count:-1}" != "0" ]]; then
    log "FAIL iteration $i incomplete PASS=$pass_count FAIL=$fail_count"
    tail -60 "$logfile" | tee -a "$OUT_DIR/matrix.log"
    echo "e2e::INCOMPLETE" >>"$OUT_DIR/summary.txt"
    exit 1
  fi
  echo "e2e::PASS:12/12" >>"$OUT_DIR/summary.txt"
  log "PASS iteration $i/$RUNS (12/12)"
done

log "=== KD3_4_E2E_NETCUP = ${RUNS}/${RUNS} × 12/12 PASS ==="
exit 0
