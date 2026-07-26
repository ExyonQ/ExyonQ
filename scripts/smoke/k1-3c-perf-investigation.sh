#!/usr/bin/env bash
# K1-3c driver: rsync repo to Netcup, run baseline or iteration bench, pull artifacts.
# Usage: K1_3C_PHASE=baseline|iter-1 bash scripts/smoke/k1-3c-perf-investigation.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${K0_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30}"
NETCUP_SSH="${NETCUP_SSH:-netcup-bench}"
NETCUP_REPO="${NETCUP_REPO:-/root/exyonq-dev-soak-src}"
RUN_ID="${K1_3C_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
PHASE="${K1_3C_PHASE:-baseline}"
LOCAL_DEST="$ROOT/benchmarks/results-dev/k1-3c-${PHASE}-${RUN_ID}"
LOG="$LOCAL_DEST/driver.log"

mkdir -p "$LOCAL_DEST"

echo "[$(date -u +%H:%M:%S)] K1-3c sync -> $NETCUP_SSH:$NETCUP_REPO phase=$PHASE"
rsync -az --exclude target --exclude .git/objects --exclude benchmarks/results --exclude benchmarks/results-dev \
  -e "ssh $SSH_OPTS" "$ROOT/" "${NETCUP_SSH}:${NETCUP_REPO}/"
echo "$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)" | ssh $SSH_OPTS "$NETCUP_SSH" "cat > ${NETCUP_REPO}/.k1-3b-commit"

chmod +x "$ROOT/scripts/smoke/k1-3c-perf-investigation-remote.sh"

echo "[$(date -u +%H:%M:%S)] K1-3c remote execute run_id=$RUN_ID phase=$PHASE"
SSH_RC=0
ssh $SSH_OPTS "$NETCUP_SSH" "bash -s" -- "$NETCUP_REPO" "$RUN_ID" "$PHASE" >>"$LOG" 2>&1 <<'REMOTE' || SSH_RC=$?
set -euo pipefail
REPO="$1"
RUN_ID="$2"
PHASE="$3"
bash "$REPO/scripts/smoke/k1-3c-perf-investigation-remote.sh" "$REPO" "$RUN_ID" "$PHASE"
REMOTE

echo "[$(date -u +%H:%M:%S)] K1-3c rsync artifacts back"
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_SSH}:${NETCUP_REPO}/benchmarks/results-dev/k1-3c-${PHASE}-${RUN_ID}/" \
  "$LOCAL_DEST/" || true

echo "=== K1-3c driver ==="
echo "phase=$PHASE run_id=$RUN_ID local_dest=$LOCAL_DEST ssh_rc=$SSH_RC"
tail -30 "$LOG" 2>/dev/null || true
exit "$SSH_RC"
