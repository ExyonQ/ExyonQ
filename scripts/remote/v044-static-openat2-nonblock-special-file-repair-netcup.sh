#!/usr/bin/env bash
# Sync and run P8R repair evidence on Netcup amd64.
set -euo pipefail

ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P8R_HOST:-netcup-bench}"
REMOTE_WS="${P8R_REMOTE_WS:-/root/exyonq-p8r-repair}"
SSH_OPTS="${SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=20}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/static-openat2-nonblock-special-file-repair/$RUN_ID"

mkdir -p "$LOCAL_EV"
REPAIR_ENTRY_HEAD="$(git -C "$ROOT" rev-parse HEAD)"
REPAIR_ENTRY_TREE="$(git -C "$ROOT" rev-parse 'HEAD^{tree}')"

ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target --exclude .git --exclude .exyonq-local \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
cd "$REMOTE_WS"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export REPAIR_ENTRY_HEAD="$REPAIR_ENTRY_HEAD"
export REPAIR_ENTRY_TREE="$REPAIR_ENTRY_TREE"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/static-openat2-nonblock-special-file-repair"
chmod +x scripts/reality/run-static-openat2-nonblock-special-file-repair-netcup.sh
bash scripts/reality/run-static-openat2-nonblock-special-file-repair-netcup.sh
REMOTE

rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/static-openat2-nonblock-special-file-repair/${RUN_ID}/" \
  "$LOCAL_EV/" || true

echo "LOCAL_EV=$LOCAL_EV"
cat "$LOCAL_EV/RESULTS.env" 2>/dev/null || true
