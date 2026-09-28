#!/usr/bin/env bash
# Mac orchestrator → Netcup PS3A-PM1 Linux drain precondition (before any file move).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS3A_PM1_PRE_RUN_ID:-pre-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps3a-pm1-precondition/$RUN_ID"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps3a-pm1-precondition/orchestrator-$RUN_ID"
REMOTE_SCRIPT="$ROOT/docs/benchmarks/platform-split/ps3a-pm1-run-precondition-linux-drain.sh"
mkdir -p "$LOCAL_DEST" "$ORCH_DIR"

echo "=== PS3A-PM1 precondition orchestrator run=$RUN_ID ===" | tee "$ORCH_DIR/orchestrator.log"
cp "$ROOT/docs/benchmarks/platform-split/ps3a-pm1-precondition/local-identity.txt" \
  "$ORCH_DIR/local-identity.txt" 2>/dev/null || true
cp "$ROOT/.ps3a-pm1-precondition.sha256" "$ORCH_DIR/critical-mac.sha256"

echo "=== rsync → $NETCUP_HOST:$NETCUP_WORKSPACE ===" | tee -a "$ORCH_DIR/orchestrator.log"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target --exclude 'target/' \
  --exclude benchmarks/results \
  --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-*' \
  --exclude 'docs/benchmarks/platform-split/ps2-results/remote-orchestrator-*' \
  --exclude 'docs/benchmarks/platform-split/ps3a-f3-i5-linux/orchestrator-*' \
  --exclude 'docs/benchmarks/platform-split/ps3a-pm1-precondition/orchestrator-*' \
  "$ROOT/" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/"

scp $SSH_OPTS "$ROOT/.ps3a-pm1-precondition.sha256" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/.ps3a-pm1-precondition.sha256"
scp $SSH_OPTS "$REMOTE_SCRIPT" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm1-run-precondition-linux-drain.sh"

echo "=== remote precondition ===" | tee -a "$ORCH_DIR/orchestrator.log"
REMOTE_RC=0
if ssh $SSH_OPTS "$NETCUP_HOST" \
  "source ~/.cargo/env 2>/dev/null; chmod +x '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps3a-pm1-run-precondition-linux-drain.sh'; PS3A_PM1_PRE_RUN_ID='$RUN_ID' bash '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps3a-pm1-run-precondition-linux-drain.sh'" \
  >"$ORCH_DIR/netcup-precondition.log" 2>&1; then
  echo "REMOTE_PRECONDITION_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
else
  REMOTE_RC=$?
  echo "REMOTE_PRECONDITION_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
fi

mkdir -p "$LOCAL_DEST"
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm1-precondition/${RUN_ID}/" \
  "$LOCAL_DEST/" || true
mkdir -p "$LOCAL_DEST/logs"
cp "$ORCH_DIR/netcup-precondition.log" "$LOCAL_DEST/logs/orchestrator-netcup-precondition.log"
cp "$ORCH_DIR/local-identity.txt" "$LOCAL_DEST/identity/local-orchestrator-identity.txt" 2>/dev/null || true

echo "PRECONDITION_PULL_COMPLETE dest=$LOCAL_DEST remote_rc=$REMOTE_RC"
if [[ -f "$LOCAL_DEST/verdicts.txt" ]]; then
  cat "$LOCAL_DEST/verdicts.txt"
else
  echo "PLATFORM_ENTRY_LINUX_DRAIN=FAIL"
  echo "missing remote verdicts.txt"
fi
exit "$REMOTE_RC"
