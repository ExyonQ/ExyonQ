#!/usr/bin/env bash
# Sync ambient CFD tree to Netcup; run Phase 6G Laravel qualification.
# PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail
ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P6G_HOST:-netcup-bench}"
REMOTE_WS="${P6G_REMOTE_WS:-/root/exyonq-p6g-laravel}"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=20"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/phase6g-additional-generic-php-app-qualification/$RUN_ID"
mkdir -p "$LOCAL_EV"

ENTRY_HEAD=$(git -C "$ROOT" rev-parse HEAD)
ENTRY_TREE=$(git -C "$ROOT" rev-parse 'HEAD^{tree}')
{
  echo "WIP=V044_PHASE6G_ADDITIONAL_GENERIC_PHP_APP_QUALIFICATION"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "RUN_ID=$RUN_ID"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

echo "[p6g] sync -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  --exclude '*.pcap' \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[p6g] remote harness RUN_ID=$RUN_ID"
ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/phase6g-additional-generic-php-app-qualification"
chmod +x scripts/reality/run-phase6g-laravel-qualification-netcup.sh
bash scripts/reality/run-phase6g-laravel-qualification-netcup.sh
REMOTE

echo "[p6g] pull evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase6g-additional-generic-php-app-qualification/${RUN_ID}/" \
  "$LOCAL_EV/" || true

echo "[p6g] done LOCAL_EV=$LOCAL_EV"
find "$LOCAL_EV" -maxdepth 3 -type f | head -80
