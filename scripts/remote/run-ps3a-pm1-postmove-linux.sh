#!/usr/bin/env bash
# Mac orchestrator → Netcup PS3A-PM1 post-move + directed P1 sync_accept.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS3A_PM1_POST_RUN_ID:-post-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps3a-pm1-postmove/$RUN_ID"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps3a-pm1-postmove/orchestrator-$RUN_ID"
mkdir -p "$LOCAL_DEST" "$ORCH_DIR"

CRITICAL=(
  crates/exyonq-platform-linux/src/sync_accept.rs
  crates/exyonq-platform-linux/src/conn_pool.rs
  crates/exyonq-platform-linux/src/lib.rs
  core/src/kernel/platform_entry.rs
  core/src/server/sync_accept_start.rs
  core/src/server/os_worker_guard.rs
  core/src/server/connection_executor.rs
  core/src/server/conn_pool.rs
  core/src/server/mod.rs
  Cargo.lock
)
: >"$ROOT/.ps3a-pm1-postmove.sha256"
for f in "${CRITICAL[@]}"; do
  if [[ -f "$f" ]]; then shasum -a 256 "$f" >>"$ROOT/.ps3a-pm1-postmove.sha256"
  else echo "MISSING  $f" >>"$ROOT/.ps3a-pm1-postmove.sha256"
  fi
done
cp "$ROOT/.ps3a-pm1-postmove.sha256" "$ORCH_DIR/critical-mac.sha256"
{
  echo "HEAD=$(git rev-parse HEAD)"
  echo "BRANCH=$(git branch --show-current)"
  echo "DATE_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "DIRTY_LINES=$(git status --short | wc -l | tr -d ' ')"
  echo "CRITICAL_MANIFEST_HASH=$(shasum -a 256 .ps3a-pm1-postmove.sha256 | awk '{print $1}')"
} | tee "$ORCH_DIR/local-identity.txt"

echo "=== rsync ===" | tee "$ORCH_DIR/orchestrator.log"
rsync -az --delete -e "ssh $SSH_OPTS" \
  --exclude target --exclude 'target/' \
  --exclude benchmarks/results \
  --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-*' \
  --exclude 'docs/benchmarks/platform-split/ps3a-pm1-postmove/orchestrator-*' \
  --exclude 'docs/benchmarks/platform-split/ps3a-pm1-precondition/orchestrator-*' \
  --exclude '.git' \
  "$ROOT/" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/"
# Ensure moved sources are not left as orphans when --delete is constrained.
ssh $SSH_OPTS "$NETCUP_HOST" "rm -f '$NETCUP_WORKSPACE/core/src/server/sync_accept.rs'"

ssh $SSH_OPTS "$NETCUP_HOST" "mkdir -p '${NETCUP_WORKSPACE}/scripts/remote' '${NETCUP_WORKSPACE}/docs/benchmarks/platform-split'"
scp $SSH_OPTS "$ROOT/.ps3a-pm1-postmove.sha256" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/.ps3a-pm1-postmove.sha256"
scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps3a-pm1-run-postmove-linux.sh" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm1-run-postmove-linux.sh"
scp $SSH_OPTS "$ROOT/scripts/remote/ps3a-pm1-directed-p1-sync.sh" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps3a-pm1-directed-p1-sync.sh"

echo "=== remote post-move (tests + P1) ===" | tee -a "$ORCH_DIR/orchestrator.log"
REMOTE_RC=0
if ssh $SSH_OPTS "$NETCUP_HOST" \
  "source ~/.cargo/env 2>/dev/null; chmod +x '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps3a-pm1-run-postmove-linux.sh' '$NETCUP_WORKSPACE/scripts/remote/ps3a-pm1-directed-p1-sync.sh'; PS3A_PM1_POST_RUN_ID='$RUN_ID' PS2_DIRECTED_REPS='${PS2_DIRECTED_REPS:-5}' bash '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps3a-pm1-run-postmove-linux.sh'" \
  >"$ORCH_DIR/netcup-postmove.log" 2>&1; then
  echo "REMOTE_POST_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
else
  REMOTE_RC=$?
  echo "REMOTE_POST_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
fi

rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm1-postmove/${RUN_ID}/" \
  "$LOCAL_DEST/" || true
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-directed/pm1-p1-${RUN_ID}/" \
  "$LOCAL_DEST/p1-directed/" 2>/dev/null || true
mkdir -p "$LOCAL_DEST/logs"
cp "$ORCH_DIR/netcup-postmove.log" "$LOCAL_DEST/logs/orchestrator-netcup-postmove.log"
cp "$ORCH_DIR/local-identity.txt" "$LOCAL_DEST/identity/local-orchestrator-identity.txt" 2>/dev/null || true

echo "POSTMOVE_PULL_COMPLETE dest=$LOCAL_DEST remote_rc=$REMOTE_RC"
[[ -f "$LOCAL_DEST/verdicts.txt" ]] && cat "$LOCAL_DEST/verdicts.txt"
exit "$REMOTE_RC"
