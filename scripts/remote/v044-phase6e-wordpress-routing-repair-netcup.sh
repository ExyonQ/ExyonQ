#!/usr/bin/env bash
# Sync CFD repair tree to Netcup; run generic PHP fixture then WordPress requalification.
# PRODUCT_MUTATION already applied locally. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail
ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P6E_HOST:-netcup-bench}"
REMOTE_WS="${P6E_REMOTE_WS:-/root/exyonq-p6e-route}"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=20"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/phase6e-wordpress-routing-repair/$RUN_ID"
mkdir -p "$LOCAL_EV"

ENTRY_HEAD=$(git -C "$ROOT" rev-parse HEAD)
ENTRY_TREE=$(git -C "$ROOT" rev-parse 'HEAD^{tree}')
{
  echo "WIP=V044_PHASE6E_WORDPRESS_ROUTING_REPAIR"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "RUN_ID=$RUN_ID"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

echo "[p6eroute] sync -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  --exclude '*.pcap' \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[p6eroute] generic PHP fixture + WordPress requal RUN_ID=$RUN_ID"
ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/phase6e-wordpress-routing-repair"
mkdir -p "\$EVIDENCE_ROOT/\$RUN_ID"
chmod +x scripts/reality/run-phase6e-generic-php-routing-fixture.sh \
  scripts/reality/run-phase6e-wordpress-qualification-netcup.sh \
  scripts/remote/v044-phase6e-wordpress-routing-repair-netcup.sh 2>/dev/null || true
set +e
bash scripts/reality/run-phase6e-generic-php-routing-fixture.sh 2>&1 | tee "\$EVIDENCE_ROOT/\$RUN_ID/generic-wrapper.log"
GC=\${PIPESTATUS[0]}
echo "GENERIC_EXIT=\$GC" | tee -a "\$EVIDENCE_ROOT/\$RUN_ID/generic-wrapper.log"
# WordPress evidence under same RUN_ID nested path expected by WP harness default —
# override to routing-repair root.
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/phase6e-wordpress-routing-repair"
# WP harness historically writes under phase6e-current-head-...; force via env if supported.
# Patch: set STAGE under routing-repair by exporting compatible vars.
export P6E_EVIDENCE_ROOT="\$EVIDENCE_ROOT"
bash scripts/reality/run-phase6e-wordpress-qualification-netcup.sh 2>&1 | tee "\$EVIDENCE_ROOT/\$RUN_ID/wordpress-wrapper.log"
WC=\${PIPESTATUS[0]}
echo "WORDPRESS_EXIT=\$WC" | tee -a "\$EVIDENCE_ROOT/\$RUN_ID/wordpress-wrapper.log"
echo "GENERIC_EXIT=\$GC" > "\$EVIDENCE_ROOT/\$RUN_ID/HARNESS_EXITS.txt"
echo "WORDPRESS_EXIT=\$WC" >> "\$EVIDENCE_ROOT/\$RUN_ID/HARNESS_EXITS.txt"
exit 0
REMOTE

echo "[p6eroute] pull evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase6e-wordpress-routing-repair/${RUN_ID}/" \
  "$LOCAL_EV/" || true
# Also pull historical WP path if harness wrote there
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase6e-current-head-real-wordpress-qualification/${RUN_ID}/" \
  "$LOCAL_EV/WORDPRESS/" 2>/dev/null || true

echo "[p6eroute] done LOCAL_EV=$LOCAL_EV"
find "$LOCAL_EV" -maxdepth 3 -type f | head -80
