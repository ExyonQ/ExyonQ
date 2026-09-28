#!/usr/bin/env bash
# Mac orchestrator → Netcup PS3A-F3-L1 (I4/I5 Linux validation). No I6. No commit.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS3A_F3_L1_RUN_ID:-l1-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps3a-f3-i5-linux/$RUN_ID"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps3a-f3-i5-linux/orchestrator-$RUN_ID"
REMOTE_SCRIPT="$ROOT/docs/benchmarks/platform-split/ps3a-f3-run-l1-linux-validation.sh"
mkdir -p "$LOCAL_DEST" "$ORCH_DIR"

echo "=== PS3A-F3-L1 orchestrator run=$RUN_ID ===" | tee "$ORCH_DIR/orchestrator.log"

CRITICAL_FILES=(
  core/src/server/accepted_connection.rs
  core/src/server/connection_errors.rs
  core/src/server/connection_executor.rs
  core/src/server/sync_accept.rs
  core/src/server/io_uring_worker.rs
  core/src/server/conn_pool.rs
  core/src/server/io.rs
  core/src/server/mod.rs
  Cargo.lock
)

{
  echo "HEAD=$(git rev-parse HEAD)"
  echo "BRANCH=$(git branch --show-current)"
  echo "DIRTY_LINES=$(git status --short | wc -l | tr -d ' ')"
  echo "STATUS_HASH=$(git status --short | LC_ALL=C sort | shasum -a 256 | awk '{print $1}')"
  echo "CARGO_LOCK_HASH=$(shasum -a 256 Cargo.lock | awk '{print $1}')"
  echo "TOOLCHAIN_LOCAL=$(rustc --version 2>/dev/null || echo missing)"
  echo "HOST_LOCAL=$(hostname)"
  echo "DATE_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$ORCH_DIR/local-identity.txt"

: >"$ROOT/.ps3a-f3-l1-critical.sha256"
for f in "${CRITICAL_FILES[@]}"; do
  if [[ -f "$f" ]]; then
    # portable sha256
    if command -v sha256sum >/dev/null; then
      sha256sum "$f" >>"$ROOT/.ps3a-f3-l1-critical.sha256"
    else
      shasum -a 256 "$f" >>"$ROOT/.ps3a-f3-l1-critical.sha256"
    fi
  else
    echo "MISSING  $f" >>"$ROOT/.ps3a-f3-l1-critical.sha256"
  fi
done
cp "$ROOT/.ps3a-f3-l1-critical.sha256" "$ORCH_DIR/critical-mac.sha256"
SOURCE_HASH="$(shasum -a 256 "$ROOT/.ps3a-f3-l1-critical.sha256" | awk '{print $1}')"
echo "CRITICAL_MANIFEST_HASH=$SOURCE_HASH" | tee -a "$ORCH_DIR/local-identity.txt"

echo "=== rsync → $NETCUP_HOST:$NETCUP_WORKSPACE ===" | tee -a "$ORCH_DIR/orchestrator.log"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target --exclude 'target/' \
  --exclude benchmarks/results \
  --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-*' \
  --exclude 'docs/benchmarks/platform-split/ps2-results/remote-orchestrator-*' \
  --exclude 'docs/benchmarks/platform-split/ps3a-f3-i5-linux/orchestrator-*' \
  "$ROOT/" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/"

scp $SSH_OPTS "$ROOT/.ps3a-f3-l1-critical.sha256" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/.ps3a-f3-l1-critical.sha256"
scp $SSH_OPTS "$REMOTE_SCRIPT" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-f3-run-l1-linux-validation.sh"
scp $SSH_OPTS "$ROOT/scripts/remote/ps2-iu1-io-uring-probe.sh" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-iu1-io-uring-probe.sh"

echo "=== remote L1 validation (tests + IU2×5) ===" | tee -a "$ORCH_DIR/orchestrator.log"
REMOTE_RC=0
if ssh $SSH_OPTS "$NETCUP_HOST" \
  "source ~/.cargo/env 2>/dev/null; chmod +x '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps3a-f3-run-l1-linux-validation.sh' '$NETCUP_WORKSPACE/scripts/remote/ps2-iu1-io-uring-probe.sh'; PS3A_F3_L1_RUN_ID='$RUN_ID' bash '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps3a-f3-run-l1-linux-validation.sh'" \
  >"$ORCH_DIR/netcup-l1.log" 2>&1; then
  echo "REMOTE_L1_RC=0" | tee -a "$ORCH_DIR/orchestrator.log"
else
  REMOTE_RC=$?
  echo "REMOTE_L1_RC=$REMOTE_RC" | tee -a "$ORCH_DIR/orchestrator.log"
fi

echo "=== pull artifacts ===" | tee -a "$ORCH_DIR/orchestrator.log"
mkdir -p "$LOCAL_DEST"
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps3a-f3-i5-linux/${RUN_ID}/" \
  "$LOCAL_DEST/" || true
# also pull probe tree if separate
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/ps3a-f3-i5-linux/" \
  "$LOCAL_DEST/ps2-probe-mirror/" 2>/dev/null || true

cp "$ORCH_DIR/netcup-l1.log" "$LOCAL_DEST/logs/orchestrator-netcup-l1.log" 2>/dev/null || mkdir -p "$LOCAL_DEST/logs" && cp "$ORCH_DIR/netcup-l1.log" "$LOCAL_DEST/logs/orchestrator-netcup-l1.log"
cp "$ORCH_DIR/local-identity.txt" "$LOCAL_DEST/identity/local-orchestrator-identity.txt" 2>/dev/null || true

echo "L1_PULL_COMPLETE dest=$LOCAL_DEST remote_rc=$REMOTE_RC"
if [[ -f "$LOCAL_DEST/verdicts.txt" ]]; then
  cat "$LOCAL_DEST/verdicts.txt"
else
  echo "WARN: verdicts.txt missing; remote may have aborted on identity" >&2
  tail -80 "$ORCH_DIR/netcup-l1.log" >&2 || true
  exit 2
fi
exit 0
