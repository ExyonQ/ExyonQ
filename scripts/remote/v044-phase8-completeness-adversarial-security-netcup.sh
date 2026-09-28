#!/usr/bin/env bash
# Sync ambient CFD tree to Netcup and run Phase 8 completeness + adversarial security.
# PARENT=STATICDUAL-B. PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail

ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P8_HOST:-netcup-bench}"
REMOTE_WS="${P8_REMOTE_WS:-/root/exyonq-p8-adv}"
SSH_OPTS="${SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=20}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/phase8-completeness-adversarial-security/$RUN_ID"

mkdir -p "$LOCAL_EV"

ENTRY_HEAD="$(git -C "$ROOT" rev-parse HEAD)"
ENTRY_TREE="$(git -C "$ROOT" rev-parse 'HEAD^{tree}')"
{
  echo "WIP=V044_PHASE8_COMPLETENESS_AND_ADVERSARIAL_SECURITY"
  echo "PARENT=STATICDUAL-B"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

echo "[p8] sync -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"

rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[p8] remote harness RUN_ID=$RUN_ID"
ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/phase8-completeness-adversarial-security"
chmod +x scripts/reality/run-phase8-completeness-adversarial-security-netcup.sh
chmod +x scripts/reality/phase8-raw-socket-client.py
bash scripts/reality/run-phase8-completeness-adversarial-security-netcup.sh
REMOTE

echo "[p8] pull evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase8-completeness-adversarial-security/${RUN_ID}/" \
  "$LOCAL_EV/" || true

date -u +"SYNC_END=%Y-%m-%dT%H:%M:%SZ" | tee -a "$LOCAL_EV/SYNC_META.txt"
echo "[p8] done LOCAL_EV=$LOCAL_EV"
find "$LOCAL_EV" -maxdepth 3 -type f | head -120
