#!/usr/bin/env bash
# Sync ambient CFD tree to Netcup; run same-binary symlink reseal harness.
# PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail
ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P6E_HOST:-netcup-bench}"
REMOTE_WS="${P6E_REMOTE_WS:-/root/exyonq-p6e-reseal}"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=20"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/phase6e-wordpress-routing-symlink-same-binary-reseal/$RUN_ID"
mkdir -p "$LOCAL_EV"

ENTRY_HEAD=$(git -C "$ROOT" rev-parse HEAD)
ENTRY_TREE=$(git -C "$ROOT" rev-parse 'HEAD^{tree}')
{
  echo "WIP=V044_PHASE6E_WORDPRESS_ROUTING_SYMLINK_SAME_BINARY_RESEAL"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "RUN_ID=$RUN_ID"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

echo "[p6e-reseal] sync -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  --exclude '*.pcap' \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[p6e-reseal] remote harness RUN_ID=$RUN_ID"
ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/phase6e-wordpress-routing-symlink-same-binary-reseal"
chmod +x scripts/reality/run-phase6e-symlink-same-binary-reseal-netcup.sh \
  scripts/reality/run-phase6e-generic-php-routing-fixture.sh \
  scripts/reality/run-phase6e-wordpress-qualification-netcup.sh
bash scripts/reality/run-phase6e-symlink-same-binary-reseal-netcup.sh
REMOTE

echo "[p6e-reseal] pull evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase6e-wordpress-routing-symlink-same-binary-reseal/${RUN_ID}/" \
  "$LOCAL_EV/" || true

echo "[p6e-reseal] done LOCAL_EV=$LOCAL_EV"
find "$LOCAL_EV" -maxdepth 3 -type f | head -100
