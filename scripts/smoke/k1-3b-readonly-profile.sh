#!/usr/bin/env bash
# K1-3b driver: rsync repo to Netcup amd64, run read-only profile, pull artifacts.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${K0_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30}"
NETCUP_SSH="${NETCUP_SSH:-netcup-bench}"
NETCUP_REPO="${NETCUP_REPO:-/root/exyonq-dev-soak-src}"
RUN_ID="${K1_3B_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_DEST="$ROOT/benchmarks/results-dev/k1-3b-profile-${RUN_ID}"
LOG="$LOCAL_DEST/driver.log"

mkdir -p "$LOCAL_DEST"

echo "[$(date -u +%H:%M:%S)] K1-3b sync -> $NETCUP_SSH:$NETCUP_REPO"
rsync -az --exclude target --exclude .git/objects --exclude benchmarks/results --exclude benchmarks/results-dev \
  -e "ssh $SSH_OPTS" "$ROOT/" "${NETCUP_SSH}:${NETCUP_REPO}/"
echo "$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)" | ssh $SSH_OPTS "$NETCUP_SSH" "cat > ${NETCUP_REPO}/.k1-3b-commit"

echo "[$(date -u +%H:%M:%S)] K1-3b remote execute run_id=$RUN_ID"
SSH_RC=0
ssh $SSH_OPTS "$NETCUP_SSH" "bash -s" -- "$NETCUP_REPO" "$RUN_ID" >>"$LOG" 2>&1 <<'REMOTE' || SSH_RC=$?
set -euo pipefail
REPO="$1"
RUN_ID="$2"
chmod +x "$REPO/scripts/smoke/k1-3b-readonly-profile-remote.sh"
export REPO="$REPO"
bash "$REPO/scripts/smoke/k1-3b-readonly-profile-remote.sh" "$REPO" "$RUN_ID"
REMOTE

echo "[$(date -u +%H:%M:%S)] K1-3b rsync artifacts back"
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_SSH}:${NETCUP_REPO}/benchmarks/results-dev/k1-3b-profile-${RUN_ID}/" \
  "$LOCAL_DEST/" || true

echo "=== K1-3b driver ==="
echo "run_id=$RUN_ID"
echo "local_dest=$LOCAL_DEST"
echo "ssh_rc=$SSH_RC"
tail -20 "$LOG" 2>/dev/null || true
exit "$SSH_RC"
