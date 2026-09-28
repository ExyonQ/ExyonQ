#!/usr/bin/env bash
# P8R repair evidence on Oracle ARM64 A1 (native).
set -euo pipefail

ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P8R_ARM_HOST:-oracle-quasar}"
REMOTE_WS="${P8R_ARM_REMOTE_WS:-/home/ubuntu/exyonq-p8r-repair-arm64}"
SSH_OPTS="${SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=25}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/static-openat2-nonblock-special-file-repair/$RUN_ID/DUAL_ARCH"

mkdir -p "$LOCAL_EV"
REPAIR_ENTRY_HEAD="$(git -C "$ROOT" rev-parse HEAD)"
REPAIR_ENTRY_TREE="$(git -C "$ROOT" rev-parse 'HEAD^{tree}')"

ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target --exclude .git --exclude .exyonq-local \
  --include 'Cargo.toml' --include 'Cargo.lock' \
  --include 'crates/exyonq-cfd-*/***' \
  --include 'crates/exyonq-fastcgi-wire/***' \
  --include 'crates/exyonq-waf*/***' \
  --include 'scripts/reality/run-static-openat2-nonblock-special-file-repair-netcup.sh' \
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

echo "ARM64_LOCAL_EV=$LOCAL_EV"
cat "$LOCAL_EV/RESULTS.env" 2>/dev/null || true
