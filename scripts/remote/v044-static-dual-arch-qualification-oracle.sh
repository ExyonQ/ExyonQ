#!/usr/bin/env bash
# Sync CFD tree to Oracle ARM64; run native Static dual-arch qualification.
# PARENT=P7R-B. PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail

ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${STATICDUAL_HOST:-oracle-quasar}"
REMOTE_WS="${STATICDUAL_REMOTE_WS:-/home/ubuntu/exyonq-staticdual-arm64}"
SSH_OPTS="${SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=25}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/static-dual-arch-qualification/$RUN_ID"

mkdir -p "$LOCAL_EV"

ENTRY_HEAD="$(git -C "$ROOT" rev-parse HEAD)"
ENTRY_TREE="$(git -C "$ROOT" rev-parse 'HEAD^{tree}')"
sha256sum \
  "$ROOT/crates/exyonq-cfd-gen/src/route_table.rs" \
  "$ROOT/crates/exyonq-cfd-gen/src/composite.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/static_serve.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/shard.rs" \
  "$ROOT/crates/exyonq-cfd-control/src/project.rs" \
  | tee "$LOCAL_EV/AMD64_REFERENCE_DIGESTS.txt"

{
  echo "WIP=V044_STATIC_DUAL_ARCH_QUALIFICATION"
  echo "PARENT=P7R-B"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

echo "[staticdual] sync -> ${HOST}:${REMOTE_WS}"
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

echo "[staticdual] remote harness RUN_ID=$RUN_ID"
# Bind AMD64-side source digests on remote before harness measures ARM64 tree.
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS/.exyonq-local/evidence/static-dual-arch-qualification/$RUN_ID/SOURCE'"
scp $SSH_OPTS "$LOCAL_EV/AMD64_REFERENCE_DIGESTS.txt" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/static-dual-arch-qualification/${RUN_ID}/SOURCE/amd64_reference_digests.txt"

ssh $SSH_OPTS "$HOST" bash -s <<REMOTE || HARNESS_RC=$?
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/static-dual-arch-qualification"
export AMD64_REF_DIGESTS="$REMOTE_WS/.exyonq-local/evidence/static-dual-arch-qualification/$RUN_ID/SOURCE/amd64_reference_digests.txt"
chmod +x scripts/reality/run-static-dual-arch-qualification-oracle.sh
bash scripts/reality/run-static-dual-arch-qualification-oracle.sh
REMOTE
HARNESS_RC="${HARNESS_RC:-0}"

echo "[staticdual] pull evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/static-dual-arch-qualification/${RUN_ID}/" \
  "$LOCAL_EV/" || true
cp -a "$LOCAL_EV/AMD64_REFERENCE_DIGESTS.txt" "$LOCAL_EV/SOURCE/amd64_reference_digests.txt" 2>/dev/null || \
  mkdir -p "$LOCAL_EV/SOURCE" && cp "$LOCAL_EV/AMD64_REFERENCE_DIGESTS.txt" "$LOCAL_EV/SOURCE/amd64_reference_digests.txt"

date -u +"SYNC_END=%Y-%m-%dT%H:%M:%SZ" | tee -a "$LOCAL_EV/SYNC_META.txt"
echo "HARNESS_RC=$HARNESS_RC" | tee -a "$LOCAL_EV/SYNC_META.txt"
echo "[staticdual] done LOCAL_EV=$LOCAL_EV"
exit "$HARNESS_RC"
