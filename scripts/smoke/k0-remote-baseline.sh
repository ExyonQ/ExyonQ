#!/usr/bin/env bash
# K0.5 remote orchestrator: rsync → build (if needed) → ExyonQ-only baseline on Netcup + Oracle.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${K0_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=20}"
RESULTS_DIR="${K0_RESULTS_DIR:-$ROOT/benchmarks/results-dev/k0.5-baseline-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$RESULTS_DIR"

NETCUP_SSH="${NETCUP_SSH:-netcup-bench}"
NETCUP_REPO="${NETCUP_REPO:-/root/exyonq-dev-soak-src}"
ORACLE_SSH="${ORACLE_SSH:-oracle-quasar}"
ORACLE_REPO="${ORACLE_REPO:-/home/ubuntu/exyonq-dev-soak-src}"

rsync_one() {
  local ssh_host="$1" repo="$2"
  rsync -az \
    --exclude target --exclude .git --exclude benchmarks/results \
    --exclude benchmarks/results-dev \
    -e "ssh $SSH_OPTS" "$ROOT/" "${ssh_host}:${repo}/"
}

run_host() {
  local label="$1" ssh_host="$2" repo="$3"
  local log="$RESULTS_DIR/${label}.log"
  echo "[$(date -u +%H:%M:%S)] $label baseline -> $log"
  ssh $SSH_OPTS "$ssh_host" "bash -s" -- "$repo" "$label" >>"$log" 2>&1 <<'REMOTE'
set -euo pipefail
REPO="$1"
LABEL="$2"
cd "$REPO"
export PATH="$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH"
export HOST_LABEL="$LABEL"
export K0_RUN_ROOT="${K0_RUN_ROOT:-$HOME/exyonq-dev-soak/k0.5-baseline}"
export K0_WARMUP_SEC="${K0_WARMUP_SEC:-5}"
export K0_MEASURE_SEC="${K0_MEASURE_SEC:-30}"
export EXYONQ_EPOLL_STATIC="${EXYONQ_EPOLL_STATIC:-1}"
export EXYONQ_EPOLL_SENDFILE="${EXYONQ_EPOLL_SENDFILE:-1}"
bash scripts/smoke/k0-exyonq-baseline.sh
REMOTE
  echo "[$(date -u +%H:%M:%S)] $label done"
}

echo "K0.5 remote baseline -> $RESULTS_DIR"
rsync_one "$NETCUP_SSH" "$NETCUP_REPO"
rsync_one "$ORACLE_SSH" "$ORACLE_REPO"

run_host "netcup-amd64" "$NETCUP_SSH" "$NETCUP_REPO" &
PID_N=$!
run_host "oracle-aarch64" "$ORACLE_SSH" "$ORACLE_REPO" &
PID_O=$!

FAIL=0
wait "$PID_N" || FAIL=1
wait "$PID_O" || FAIL=1

echo ""
echo "=== K0.5 logs: $RESULTS_DIR ==="
ls -la "$RESULTS_DIR"
exit "$FAIL"
