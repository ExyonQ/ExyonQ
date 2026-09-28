#!/usr/bin/env bash
# Mac orchestrator → Netcup PS3A-PM5 final validation (units + IU2 + directed P1 + contracts).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS3A_PM5_DIRECTED_RUN_ID:-pm5-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps3a-pm5/$RUN_ID"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps3a-pm5/orchestrator-$RUN_ID"
mkdir -p "$LOCAL_DEST" "$ORCH_DIR"

{
  echo "HEAD=$(git rev-parse HEAD)"
  echo "BRANCH=$(git branch --show-current)"
  echo "DATE_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "DIRTY_LINES=$(git status --short | wc -l | tr -d ' ')"
  echo "BASELINE_INDEX_TOUCHED=NO"
  echo "FULL_R7C=NO"
} | tee "$ORCH_DIR/local-identity.txt"

echo "=== rsync ===" | tee "$ORCH_DIR/orchestrator.log"
rsync -az --delete -e "ssh $SSH_OPTS" \
  --exclude target --exclude 'target/' \
  --exclude benchmarks/results \
  --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-*' \
  --exclude 'docs/benchmarks/platform-split/ps3a-*/orchestrator-*' \
  --exclude '.git' \
  "$ROOT/" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/"

ssh $SSH_OPTS "$NETCUP_HOST" \
  "rm -f '$NETCUP_WORKSPACE/core/src/server/epoll_worker.rs' \
         '$NETCUP_WORKSPACE/core/src/server/io_uring_worker.rs' \
         '$NETCUP_WORKSPACE/core/src/server/sync_accept.rs'"

ssh $SSH_OPTS "$NETCUP_HOST" "mkdir -p '${NETCUP_WORKSPACE}/scripts/remote'"
for f in ps3a-pm5-directed-p1.sh ps3a-pm5-summarize-directed.py ps3a-pm2-iu2-protector.sh; do
  scp $SSH_OPTS "$ROOT/scripts/remote/$f" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/$f"
done

REMOTE_RC=0
echo "=== remote linux unit + contract gates ===" | tee -a "$ORCH_DIR/orchestrator.log"
if ssh $SSH_OPTS "$NETCUP_HOST" bash -s <<EOF
set -euo pipefail
source ~/.cargo/env 2>/dev/null || true
cd '$NETCUP_WORKSPACE'
export PATH="\${HOME}/.cargo/bin:/usr/local/cargo/bin:\${PATH}"
mkdir -p docs/benchmarks/platform-split/ps3a-pm5/$RUN_ID/logs
{
  cargo test -p exyonq-core --lib epoll_contract
  cargo test -p exyonq-core --lib ps1a
  cargo test -p exyonq-core --lib ps1b
  cargo test -p exyonq-core --lib ps1c
  cargo test -p exyonq-core --lib ps3a
  cargo test -p exyonq-platform-linux
  cargo test -p exyonq-kernel-contract-consumer
} 2>&1 | tee docs/benchmarks/platform-split/ps3a-pm5/$RUN_ID/logs/linux-unit-gates.log
EOF
then
  echo "REMOTE_UNIT_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
else
  REMOTE_RC=$?
  echo "REMOTE_UNIT_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
fi

if [[ "$REMOTE_RC" -eq 0 ]]; then
  echo "=== remote IU2 protector ===" | tee -a "$ORCH_DIR/orchestrator.log"
  if ssh $SSH_OPTS "$NETCUP_HOST" \
    "source ~/.cargo/env 2>/dev/null; chmod +x '$NETCUP_WORKSPACE/scripts/remote/ps3a-pm2-iu2-protector.sh'; PS3A_PM2_IU2_RUN_ID='pm5-iu2-$RUN_ID' PS3A_PM2_COMPOSE_PROJECT='exyonq-ps3a-pm5-iu2' bash '$NETCUP_WORKSPACE/scripts/remote/ps3a-pm2-iu2-protector.sh'" \
    >"$ORCH_DIR/netcup-iu2.log" 2>&1; then
    echo "REMOTE_IU2_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
  else
    REMOTE_RC=$?
    echo "REMOTE_IU2_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
  fi
fi

if [[ "$REMOTE_RC" -eq 0 ]]; then
  echo "=== remote directed P1 + contracts ===" | tee -a "$ORCH_DIR/orchestrator.log"
  if ssh $SSH_OPTS "$NETCUP_HOST" \
    "source ~/.cargo/env 2>/dev/null; chmod +x '$NETCUP_WORKSPACE/scripts/remote/ps3a-pm5-directed-p1.sh'; PS3A_PM5_DIRECTED_RUN_ID='$RUN_ID' bash '$NETCUP_WORKSPACE/scripts/remote/ps3a-pm5-directed-p1.sh'" \
    >"$ORCH_DIR/netcup-directed.log" 2>&1; then
    echo "REMOTE_DIRECTED_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
  else
    REMOTE_RC=$?
    echo "REMOTE_DIRECTED_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
  fi
fi

rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm5/${RUN_ID}/" \
  "$LOCAL_DEST/" || true
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm2-iu2/pm5-iu2-${RUN_ID}/" \
  "$LOCAL_DEST/iu2/" || true
cp "$ORCH_DIR/"*.log "$LOCAL_DEST/logs/" 2>/dev/null || true
cp "$ORCH_DIR/"*.txt "$LOCAL_DEST/env/" 2>/dev/null || true

echo "PM5_PULL_COMPLETE dest=$LOCAL_DEST remote_rc=$REMOTE_RC"
exit "$REMOTE_RC"
