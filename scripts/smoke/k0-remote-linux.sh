#!/usr/bin/env bash
# K0.4 remote Linux smoke: rsync → build → default + epoll_sendfile profiles.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${K0_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=20}"
RESULTS_DIR="${K0_RESULTS_DIR:-$ROOT/benchmarks/results-dev/k0.4-smoke-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$RESULTS_DIR"

run_host() {
  local label="$1" ssh_host="$2" repo="$3"
  local log="$RESULTS_DIR/${label}.log"
  echo "[$(date -u +%H:%M:%S)] $label: rsync+build+smoke -> $log"
  ssh $SSH_OPTS "$ssh_host" "bash -s" -- "$repo" "$label" >>"$log" 2>&1 <<'REMOTE'
set -euo pipefail
REPO="$1"
LABEL="$2"
cd "$REPO"
export PATH="$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH"
echo "=== $LABEL $(uname -sm) commit=$(git rev-parse --short HEAD 2>/dev/null || echo unknown) ==="
cargo build -p exyonq -q
export K0_HOST_LABEL="$LABEL"
echo "--- profile default ---"
bash scripts/smoke/k0-core-smoke-matrix.sh
echo "--- profile epoll_sendfile ---"
EXYONQ_EPOLL_STATIC=1 EXYONQ_EPOLL_SENDFILE=1 bash scripts/smoke/k0-core-smoke-matrix.sh
REMOTE
  echo "[$(date -u +%H:%M:%S)] $label: done"
}

rsync_one() {
  local ssh_host="$1" repo="$2"
  rsync -az \
    --exclude target --exclude .git --exclude benchmarks/results \
    --exclude benchmarks/results-dev \
    -e "ssh $SSH_OPTS" "$ROOT/" "${ssh_host}:${repo}/"
}

NETCUP_SSH="${NETCUP_SSH:-netcup-bench}"
NETCUP_REPO="${NETCUP_REPO:-/root/exyonq-dev-soak-src}"
ORACLE_SSH="${ORACLE_SSH:-oracle-quasar}"
ORACLE_REPO="${ORACLE_REPO:-/home/ubuntu/exyonq-dev-soak-src}"

echo "K0.4 remote smoke -> $RESULTS_DIR"
echo "rsync Netcup..."
rsync_one "$NETCUP_SSH" "$NETCUP_REPO"
echo "rsync Oracle..."
rsync_one "$ORACLE_SSH" "$ORACLE_REPO"

run_host "netcup-amd64" "$NETCUP_SSH" "$NETCUP_REPO" &
PID_N=$!
run_host "oracle-aarch64" "$ORACLE_SSH" "$ORACLE_REPO" &
PID_O=$!

FAIL=0
wait "$PID_N" || FAIL=1
wait "$PID_O" || FAIL=1

echo ""
echo "=== logs in $RESULTS_DIR ==="
ls -la "$RESULTS_DIR"
exit "$FAIL"
