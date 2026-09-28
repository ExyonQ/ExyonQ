#!/usr/bin/env bash
# Sync ambient CFD tree to Netcup and run Phase 7 three-handler requalification.
# PARENT=P7STATICIMPL-B. PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail

ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P7R_HOST:-netcup-bench}"
REMOTE_WS="${P7R_REMOTE_WS:-/root/exyonq-p7r-coexist}"
SSH_OPTS="${SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=20}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/phase7-static-proxy-fastcgi-requalification/$RUN_ID"

mkdir -p "$LOCAL_EV"

ENTRY_HEAD="$(git -C "$ROOT" rev-parse HEAD)"
ENTRY_TREE="$(git -C "$ROOT" rev-parse 'HEAD^{tree}')"
{
  echo "WIP=V044_PHASE7_STATIC_PROXY_FASTCGI_REQUALIFICATION"
  echo "PARENT=P7STATICIMPL-B"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

echo "[p7r] sync -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"

rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  --include 'Cargo.toml' \
  --include 'Cargo.lock' \
  --include 'crates/exyonq-cfd-*/***' \
  --include 'crates/exyonq-fastcgi-wire/***' \
  --include 'crates/exyonq-waf*/***' \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[p7r] remote harness RUN_ID=$RUN_ID"
ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/phase7-static-proxy-fastcgi-requalification"
chmod +x scripts/reality/run-phase7-static-proxy-fastcgi-requalification-netcup.sh
bash scripts/reality/run-phase7-static-proxy-fastcgi-requalification-netcup.sh
REMOTE

echo "[p7r] pull evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase7-static-proxy-fastcgi-requalification/${RUN_ID}/" \
  "$LOCAL_EV/" || true

date -u +"SYNC_END=%Y-%m-%dT%H:%M:%SZ" | tee -a "$LOCAL_EV/SYNC_META.txt"
echo "[p7r] done LOCAL_EV=$LOCAL_EV"
find "$LOCAL_EV" -maxdepth 3 -type f | head -80
