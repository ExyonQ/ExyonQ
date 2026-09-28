#!/usr/bin/env bash
# Sync ambient CFD tree to Netcup and run native static filesystem handler proof.
# PARENT=P7STATICADM-B. This is not Phase 7 requalification.
# PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail

ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${CFD_STATIC_HOST:-netcup-bench}"
REMOTE_WS="${CFD_STATIC_REMOTE_WS:-/root/exyonq-cfd-static-impl}"
SSH_OPTS="${SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=20}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/cfd-native-static-filesystem-handler-implementation/$RUN_ID"

mkdir -p "$LOCAL_EV"

ENTRY_HEAD="$(git -C "$ROOT" rev-parse HEAD)"
ENTRY_TREE="$(git -C "$ROOT" rev-parse 'HEAD^{tree}')"
{
  echo "WIP=V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_IMPLEMENTATION"
  echo "PARENT=P7STATICADM-B"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

echo "[cfd-static] sync -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"

# Keep the Phase 6G rsync shape: sync the ambient lab tree, exclude bulky/local
# state, and make sure all CFD/static route crates are present on the host.
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  --exclude '*.pcap' \
  --include 'Cargo.toml' \
  --include 'Cargo.lock' \
  --include 'crates/exyonq-cfd-*/***' \
  --include 'crates/exyonq-fastcgi-wire/***' \
  --include 'crates/exyonq-waf*/***' \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[cfd-static] remote harness RUN_ID=$RUN_ID"
ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/cfd-native-static-filesystem-handler-implementation"
chmod +x scripts/reality/run-cfd-native-static-filesystem-handler-netcup.sh
bash scripts/reality/run-cfd-native-static-filesystem-handler-netcup.sh
REMOTE

echo "[cfd-static] pull evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/cfd-native-static-filesystem-handler-implementation/${RUN_ID}/" \
  "$LOCAL_EV/" || true

date -u +"SYNC_END=%Y-%m-%dT%H:%M:%SZ" | tee -a "$LOCAL_EV/SYNC_META.txt"
echo "[cfd-static] done LOCAL_EV=$LOCAL_EV"
find "$LOCAL_EV" -maxdepth 3 -type f | head -80
