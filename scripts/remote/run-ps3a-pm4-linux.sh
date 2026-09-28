#!/usr/bin/env bash
# Mac orchestrator → Netcup PS3A-PM4 short functional check + Linux unit gates.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS3A_PM4_FUNC_RUN_ID:-pm4-func-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps3a-pm4/$RUN_ID"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps3a-pm4/orchestrator-$RUN_ID"
SKIP_UNITS="${SKIP_UNITS:-0}"
SKIP_RSYNC="${SKIP_RSYNC:-0}"
mkdir -p "$LOCAL_DEST" "$ORCH_DIR"

{
  echo "HEAD=$(git rev-parse HEAD)"
  echo "BRANCH=$(git branch --show-current)"
  echo "DATE_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "DIRTY_LINES=$(git status --short | wc -l | tr -d ' ')"
  echo "BASELINE_INDEX_TOUCHED=NO"
  echo "SKIP_UNITS=$SKIP_UNITS"
  echo "SKIP_RSYNC=$SKIP_RSYNC"
} | tee "$ORCH_DIR/local-identity.txt"

if [[ "$SKIP_RSYNC" != "1" ]]; then
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
else
  echo "=== skip rsync (SKIP_RSYNC=1) ===" | tee "$ORCH_DIR/orchestrator.log"
fi

ssh $SSH_OPTS "$NETCUP_HOST" "mkdir -p '${NETCUP_WORKSPACE}/scripts/remote' '${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm4'"
scp $SSH_OPTS "$ROOT/scripts/remote/ps3a-pm4-functional.sh" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps3a-pm4-functional.sh"

REMOTE_RC=0
if [[ "$SKIP_UNITS" != "1" ]]; then
  echo "=== remote linux unit gates ===" | tee -a "$ORCH_DIR/orchestrator.log"
  if ssh $SSH_OPTS "$NETCUP_HOST" bash -s <<EOF
set -euo pipefail
source ~/.cargo/env 2>/dev/null || true
cd '$NETCUP_WORKSPACE'
export PATH="\${HOME}/.cargo/bin:/usr/local/cargo/bin:\${PATH}"
mkdir -p docs/benchmarks/platform-split/ps3a-pm4/$RUN_ID/logs
{
  cargo test -p exyonq-core --lib epoll_contract
  cargo test -p exyonq-core --lib ps1a
  cargo test -p exyonq-core --lib ps1b
  cargo test -p exyonq-core --lib ps3a
  cargo test -p exyonq-platform-linux
} 2>&1 | tee docs/benchmarks/platform-split/ps3a-pm4/$RUN_ID/logs/linux-unit-gates.log
EOF
  then
    echo "REMOTE_UNIT_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
  else
    REMOTE_RC=$?
    echo "REMOTE_UNIT_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
  fi
else
  echo "=== skip units (SKIP_UNITS=1) ===" | tee -a "$ORCH_DIR/orchestrator.log"
fi

if [[ "$REMOTE_RC" -eq 0 ]]; then
  echo "=== remote E2E ===" | tee -a "$ORCH_DIR/orchestrator.log"
  scp $SSH_OPTS "$ROOT/scripts/remote/ps3a-pm4-functional.sh" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps3a-pm4-functional.sh"
  if ssh $SSH_OPTS "$NETCUP_HOST" \
    "source ~/.cargo/env 2>/dev/null; chmod +x '$NETCUP_WORKSPACE/scripts/remote/ps3a-pm4-functional.sh'; PS3A_PM4_FUNC_RUN_ID='$RUN_ID' bash '$NETCUP_WORKSPACE/scripts/remote/ps3a-pm4-functional.sh'" \
    >"$ORCH_DIR/netcup-e2e.log" 2>&1; then
    echo "REMOTE_E2E_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
  else
    REMOTE_RC=$?
    echo "REMOTE_E2E_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
  fi
fi

rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm4/${RUN_ID}/" \
  "$LOCAL_DEST/" || true
cp "$ORCH_DIR/netcup-e2e.log" "$LOCAL_DEST/logs/orchestrator-netcup-e2e.log" 2>/dev/null || true
cp "$ORCH_DIR/"*.txt "$LOCAL_DEST/env/" 2>/dev/null || true

echo "E2E_PULL_COMPLETE dest=$LOCAL_DEST remote_rc=$REMOTE_RC"
exit "$REMOTE_RC"
