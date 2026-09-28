#!/usr/bin/env bash
# K0.6 remote orchestrator: rsync → controlled compare on Netcup + Oracle (parallel by default).
#
# Usage:
#   K0_RUN_STAMP="20260704T120000Z" bash scripts/smoke/k0-remote-compare.sh
#   K0_PARALLEL=0 ...   # sequential (legacy ~2× wall time)
#   K0_PARALLEL=1 ... # default: both hosts concurrently (~max(AMD64, ARM) wall time)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${K0_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=20}"
STAMP="${K0_RUN_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_DIR="${K0_RESULTS_DIR:-$ROOT/benchmarks/results-dev/k0.6-compare-${STAMP}}"
PARALLEL="${K0_PARALLEL:-1}"
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

run_compare() {
  local label="$1" ssh_host="$2" repo="$3"
  local log="$RESULTS_DIR/${label}.log"
  echo "[$(date -u +%H:%M:%S)] $label compare -> $log"
  ssh $SSH_OPTS "$ssh_host" "bash -s" -- "$repo" "$label" "$STAMP" >>"$log" 2>&1 <<'REMOTE'
set -euo pipefail
REPO="$1"
LABEL="$2"
STAMP="$3"
export K0_RUN_STAMP="$STAMP"
export K0_COMPARE_ROOT="${K0_COMPARE_ROOT:-$HOME/exyonq-dev-soak/k0.6-compare}"
bash "$REPO/scripts/smoke/k0-compare-host.sh" "$REPO" "$LABEL"
REMOTE
  local rc=$?
  echo "[$(date -u +%H:%M:%S)] $label exit=$rc"
  return "$rc"
}

pull_artifacts() {
  local label="$1" ssh_host="$2" repo="$3"
  local remote="$repo/benchmarks/results-dev/k0.6-compare/${STAMP}-${label}"
  local local="$RESULTS_DIR/${label}"
  mkdir -p "$local"
  rsync -az -e "ssh $SSH_OPTS" "${ssh_host}:${remote}/" "$local/" || true
}

run_host_pipeline() {
  local label="$1" ssh_host="$2" repo="$3"
  local rc=0
  run_compare "$label" "$ssh_host" "$repo" || rc=$?
  pull_artifacts "$label" "$ssh_host" "$repo"
  return "$rc"
}

echo "K0.6 remote compare stamp=$STAMP parallel=$PARALLEL -> $RESULTS_DIR"

if [[ "$PARALLEL" == "1" ]]; then
  rsync_one "$NETCUP_SSH" "$NETCUP_REPO" &
  PID_RSYNC_N=$!
  rsync_one "$ORACLE_SSH" "$ORACLE_REPO" &
  PID_RSYNC_O=$!
  RSYNC_FAIL=0
  wait "$PID_RSYNC_N" || RSYNC_FAIL=1
  wait "$PID_RSYNC_O" || RSYNC_FAIL=1
  if [[ "$RSYNC_FAIL" -ne 0 ]]; then
    echo "WARN: one or more rsync steps failed; continuing with compare" >&2
  fi

  run_host_pipeline "netcup-amd64" "$NETCUP_SSH" "$NETCUP_REPO" &
  PID_N=$!
  run_host_pipeline "oracle-aarch64" "$ORACLE_SSH" "$ORACLE_REPO" &
  PID_O=$!

  RC_N=0
  RC_O=0
  wait "$PID_N" || RC_N=$?
  wait "$PID_O" || RC_O=$?

  FAIL=0
  [[ "$RC_N" -eq 0 ]] || FAIL=1
  [[ "$RC_O" -eq 0 ]] || FAIL=1
else
  rsync_one "$NETCUP_SSH" "$NETCUP_REPO"
  rsync_one "$ORACLE_SSH" "$ORACLE_REPO"

  FAIL=0
  run_host_pipeline "netcup-amd64" "$NETCUP_SSH" "$NETCUP_REPO" || FAIL=1
  run_host_pipeline "oracle-aarch64" "$ORACLE_SSH" "$ORACLE_REPO" || FAIL=1
fi

echo ""
echo "=== K0.6 orchestrator done (parallel=$PARALLEL fail=$FAIL) ==="
ls -la "$RESULTS_DIR"
exit "$FAIL"
