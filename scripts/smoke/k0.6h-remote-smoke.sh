#!/usr/bin/env bash
# K0.6h remote harness smoke on Netcup (linux docker gate).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${K0_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30}"
STAMP="${K0H_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_DIR="${K0H_RESULTS_DIR:-$ROOT/benchmarks/results-dev/k0.6h-smoke-${STAMP}}"
mkdir -p "$RESULTS_DIR"

NETCUP_SSH="${NETCUP_SSH:-netcup-bench}"
NETCUP_REPO="${NETCUP_REPO:-/root/exyonq-dev-soak-src}"

rsync -az \
  --exclude target --exclude .git --exclude benchmarks/results \
  --exclude benchmarks/results-dev \
  -e "ssh $SSH_OPTS" "$ROOT/" "${NETCUP_SSH}:${NETCUP_REPO}/"

LOG="$RESULTS_DIR/netcup-smoke.log"
echo "[$(date -u +%H:%M:%S)] K0.6h smoke on netcup -> $LOG"
ssh $SSH_OPTS "$NETCUP_SSH" "bash -s" -- "$NETCUP_REPO" "$STAMP" >>"$LOG" 2>&1 <<'REMOTE'
set -euo pipefail
REPO="$1"
STAMP="$2"
cd "$REPO"
export PATH="$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH"
export BENCH_PROVIDER=netcup
export BENCH_SHAPE="RS 2000 G12"
export BENCH_ARCH=x86_64
export BENCH_CLAIM_LEVEL=official_candidate
export K0H_STAMP="$STAMP"
export K0H_SMOKE_DIR="$REPO/benchmarks/results-dev/k0.6h-smoke-${STAMP}"
bash scripts/smoke/k0.6h-harness-smoke.sh "$REPO"
REMOTE

rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_SSH}:${NETCUP_REPO}/benchmarks/results-dev/k0.6h-smoke-${STAMP}/" \
  "$RESULTS_DIR/" || true

echo "=== K0.6h smoke done ==="
cat "$RESULTS_DIR/gate.txt" 2>/dev/null || cat "$RESULTS_DIR/smoke.log" | tail -5
