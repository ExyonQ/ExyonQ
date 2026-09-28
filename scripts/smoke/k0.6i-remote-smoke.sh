#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${K0_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30}"
STAMP="${K0I_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_DIR="${K0I_RESULTS_DIR:-$ROOT/benchmarks/results-dev/k0.6i-smoke-${STAMP}}"
mkdir -p "$RESULTS_DIR"

NETCUP_SSH="${NETCUP_SSH:-netcup-bench}"
NETCUP_REPO="${NETCUP_REPO:-/root/exyonq-dev-soak-src}"

rsync -az --exclude target --exclude .git --exclude benchmarks/results --exclude benchmarks/results-dev \
  -e "ssh $SSH_OPTS" "$ROOT/" "${NETCUP_SSH}:${NETCUP_REPO}/"

LOG="$RESULTS_DIR/netcup-smoke.log"
echo "[$(date -u +%H:%M:%S)] K0.6i P7 loadgen smoke -> $LOG"
SSH_RC=0
ssh $SSH_OPTS "$NETCUP_SSH" "bash -s" -- "$NETCUP_REPO" "$STAMP" >>"$LOG" 2>&1 <<'REMOTE' || SSH_RC=$?
set -euo pipefail
REPO="$1"
STAMP="$2"
cd "$REPO"
export PATH="$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH"
export BENCH_PROVIDER=netcup
export BENCH_ARCH=x86_64
export K0I_STAMP="$STAMP"
export K0I_SMOKE_DIR="$REPO/benchmarks/results-dev/k0.6i-smoke-${STAMP}"
bash scripts/smoke/k0.6i-p7-loadgen-smoke.sh "$REPO"
REMOTE

rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_SSH}:${NETCUP_REPO}/benchmarks/results-dev/k0.6i-smoke-${STAMP}/" \
  "$RESULTS_DIR/" || true

echo "=== K0.6i smoke ==="
cat "$RESULTS_DIR/gate.txt" 2>/dev/null || tail -5 "$LOG"
exit "$SSH_RC"
