#!/usr/bin/env bash
# Mac orchestrator → Netcup PS3A-PM2 post-move + IU2 protector.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS3A_PM2_POST_RUN_ID:-post-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps3a-pm2-postmove/$RUN_ID"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps3a-pm2-postmove/orchestrator-$RUN_ID"
mkdir -p "$LOCAL_DEST" "$ORCH_DIR"

CRITICAL=(
  crates/exyonq-platform-linux/src/io_uring_worker.rs
  crates/exyonq-platform-linux/src/linux_bind.rs
  crates/exyonq-platform-linux/src/sync_accept.rs
  crates/exyonq-platform-linux/src/lib.rs
  core/src/server/io_uring_start.rs
  core/src/server/ps2_iu2_tests.rs
  core/src/server/mod.rs
  core/src/kernel/platform_entry.rs
  Cargo.lock
)
: >"$ROOT/.ps3a-pm2-postmove.sha256"
for f in "${CRITICAL[@]}"; do
  if [[ -f "$f" ]]; then shasum -a 256 "$f" >>"$ROOT/.ps3a-pm2-postmove.sha256"
  else echo "MISSING  $f" >>"$ROOT/.ps3a-pm2-postmove.sha256"
  fi
done
cp "$ROOT/.ps3a-pm2-postmove.sha256" "$ORCH_DIR/critical-mac.sha256"
{
  echo "HEAD=$(git rev-parse HEAD)"
  echo "BRANCH=$(git branch --show-current)"
  echo "DATE_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "DIRTY_LINES=$(git status --short | wc -l | tr -d ' ')"
  echo "CRITICAL_MANIFEST_HASH=$(shasum -a 256 .ps3a-pm2-postmove.sha256 | awk '{print $1}')"
} | tee "$ORCH_DIR/local-identity.txt"

echo "=== rsync ===" | tee "$ORCH_DIR/orchestrator.log"
rsync -az --delete -e "ssh $SSH_OPTS" \
  --exclude target --exclude 'target/' \
  --exclude benchmarks/results \
  --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-*' \
  --exclude 'docs/benchmarks/platform-split/ps3a-pm2-postmove/orchestrator-*' \
  --exclude 'docs/benchmarks/platform-split/ps3a-pm1-*/orchestrator-*' \
  --exclude '.git' \
  "$ROOT/" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/"
ssh $SSH_OPTS "$NETCUP_HOST" \
  "rm -f '$NETCUP_WORKSPACE/core/src/server/io_uring_worker.rs' '$NETCUP_WORKSPACE/core/src/server/sync_accept.rs'"

ssh $SSH_OPTS "$NETCUP_HOST" "mkdir -p '${NETCUP_WORKSPACE}/scripts/remote' '${NETCUP_WORKSPACE}/docs/benchmarks/platform-split'"
scp $SSH_OPTS "$ROOT/.ps3a-pm2-postmove.sha256" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/.ps3a-pm2-postmove.sha256"
scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps3a-pm2-run-postmove-linux.sh" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm2-run-postmove-linux.sh"
scp $SSH_OPTS "$ROOT/scripts/remote/ps3a-pm2-iu2-protector.sh" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps3a-pm2-iu2-protector.sh"

echo "=== remote post-move ===" | tee -a "$ORCH_DIR/orchestrator.log"
REMOTE_RC=0
if ssh $SSH_OPTS "$NETCUP_HOST" \
  "source ~/.cargo/env 2>/dev/null; chmod +x '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps3a-pm2-run-postmove-linux.sh' '$NETCUP_WORKSPACE/scripts/remote/ps3a-pm2-iu2-protector.sh'; PS3A_PM2_POST_RUN_ID='$RUN_ID' bash '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps3a-pm2-run-postmove-linux.sh'" \
  >"$ORCH_DIR/netcup-postmove.log" 2>&1; then
  echo "REMOTE_POST_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
else
  REMOTE_RC=$?
  echo "REMOTE_POST_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
fi

rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm2-postmove/${RUN_ID}/" \
  "$LOCAL_DEST/" || true
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-pm2-iu2/" \
  "$LOCAL_DEST/iu2-mirror/" 2>/dev/null || true
mkdir -p "$LOCAL_DEST/logs" "$LOCAL_DEST/identity"
cp "$ORCH_DIR/netcup-postmove.log" "$LOCAL_DEST/logs/orchestrator-netcup-postmove.log"
cp "$ORCH_DIR/local-identity.txt" "$LOCAL_DEST/identity/local-orchestrator-identity.txt" 2>/dev/null || true

echo "POSTMOVE_PULL_COMPLETE dest=$LOCAL_DEST remote_rc=$REMOTE_RC"
[[ -f "$LOCAL_DEST/verdicts.txt" ]] && cat "$LOCAL_DEST/verdicts.txt"
exit "$REMOTE_RC"
