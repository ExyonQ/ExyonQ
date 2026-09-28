#!/usr/bin/env bash
# Sync attributable CFD+workspace tree to Netcup and run Phase 6E WordPress qualification.
# PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail
ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P6E_HOST:-netcup-bench}"
REMOTE_WS="${P6E_REMOTE_WS:-/root/exyonq-p6e-wp}"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=20"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/phase6e-current-head-real-wordpress-qualification/$RUN_ID"
mkdir -p "$LOCAL_EV"

ENTRY_HEAD=$(git -C "$ROOT" rev-parse HEAD)
ENTRY_TREE=$(git -C "$ROOT" rev-parse 'HEAD^{tree}')
{
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "RUN_ID=$RUN_ID"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

echo "[p6e] sync workspace -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  --exclude '*.pcap' \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[p6e] remote harness RUN_ID=$RUN_ID"
ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/phase6e-current-head-real-wordpress-qualification"
mkdir -p "\$EVIDENCE_ROOT/\$RUN_ID"
chmod +x scripts/reality/run-phase6e-wordpress-qualification-netcup.sh
set +e
bash scripts/reality/run-phase6e-wordpress-qualification-netcup.sh 2>&1 | tee "\$EVIDENCE_ROOT/\$RUN_ID/harness-wrapper.log"
HC=\${PIPESTATUS[0]}
echo "HARNESS_EXIT=\$HC" | tee -a "\$EVIDENCE_ROOT/\$RUN_ID/harness-wrapper.log"
exit 0
REMOTE

echo "[p6e] pull evidence"
mkdir -p "$ROOT/.exyonq-local/evidence/phase6e-current-head-real-wordpress-qualification"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase6e-current-head-real-wordpress-qualification/${RUN_ID}/" \
  "$LOCAL_EV/"

echo "[p6e] done LOCAL_EV=$LOCAL_EV"
ls -la "$LOCAL_EV" | head -40
test -f "$LOCAL_EV/SUMMARY.txt" && cat "$LOCAL_EV/SUMMARY.txt"
