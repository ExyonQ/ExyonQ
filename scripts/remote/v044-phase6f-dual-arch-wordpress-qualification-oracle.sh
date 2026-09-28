#!/usr/bin/env bash
# Sync ambient CFD tree to Oracle ARM64; run same-binary WordPress qualification harness.
# PRODUCT_MUTATION=NO. Native aarch64 only. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail
ROOT="/Volumes/Lexar/Cursor/exyonq-laboratorio"
HOST="${P6F_HOST:-oracle-quasar}"
REMOTE_WS="${P6F_REMOTE_WS:-/home/ubuntu/exyonq-p6f-arm64}"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=25"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/phase6f-dual-arch-wordpress-qualification/$RUN_ID"
mkdir -p "$LOCAL_EV"

ENTRY_HEAD=$(git -C "$ROOT" rev-parse HEAD)
ENTRY_TREE=$(git -C "$ROOT" rev-parse 'HEAD^{tree}')
{
  echo "WIP=V044_PHASE6F_DUAL_ARCH_WORDPRESS_QUALIFICATION"
  echo "PARENT_TERMINAL=P6EROUTE-A"
  echo "AMD64_EVIDENCE=REUSED_P6EROUTE_A"
  echo "AMD64_RUN=20260831T210044Z"
  echo "AMD64_BINARY_SHA256=5d4b6bdf6c6439f0b98656e1e84c83ea6a5cd0640490bf310e9e8af7197b367b"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "REMOTE_WS=$REMOTE_WS"
  echo "HOST=$HOST"
  echo "RUN_ID=$RUN_ID"
  date -u +"SYNC_START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$LOCAL_EV/SYNC_META.txt"

# Bind CFD source digests for cross-arch equivalence
cp "$ROOT/.exyonq-local/evidence/phase6f-dual-arch-wordpress-qualification/pre-mutation/cfd_file_sha256.txt" \
  "$LOCAL_EV/AMD64_SIDE_CFD_DIGESTS.txt" 2>/dev/null || \
  sha256sum \
    "$ROOT/crates/exyonq-cfd-dataplane/src/fcgi_route.rs" \
    "$ROOT/crates/exyonq-cfd-dataplane/src/shard.rs" \
    "$ROOT/crates/exyonq-cfd-dataplane/src/fcgi_exec.rs" \
    "$ROOT/crates/exyonq-cfd-gen/src/route_table.rs" \
    "$ROOT/crates/exyonq-cfd-gen/src/composite.rs" \
    "$ROOT/crates/exyonq-cfd-control/src/project.rs" \
    | tee "$LOCAL_EV/AMD64_SIDE_CFD_DIGESTS.txt"

echo "[p6f] sync -> ${HOST}:${REMOTE_WS}"
ssh $SSH_OPTS "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e "ssh $SSH_OPTS" \
  --exclude target \
  --exclude .git \
  --exclude .exyonq-local \
  --exclude benchmarks/results \
  --exclude node_modules \
  --exclude '*.pcap' \
  "$ROOT/" "${HOST}:${REMOTE_WS}/"

echo "[p6f] remote native ARM64 harness RUN_ID=$RUN_ID"
ssh $SSH_OPTS "$HOST" bash -s <<REMOTE
set -euo pipefail
source "\${HOME}/.cargo/env" 2>/dev/null || true
export PATH="\${HOME}/.cargo/bin:/root/.cargo/bin:\${PATH}"
cd "$REMOTE_WS"
export ENTRY_HEAD="$ENTRY_HEAD"
export ENTRY_TREE="$ENTRY_TREE"
export EXYONQ_ROOT="$REMOTE_WS"
export RUN_ID="$RUN_ID"
export EVIDENCE_ROOT="$REMOTE_WS/.exyonq-local/evidence/phase6f-dual-arch-wordpress-qualification"
# Ubuntu non-root MariaDB/systemctl access
export MYSQL_ADMIN_CLI="sudo mysql"
export SYSTEMCTL_CLI="sudo systemctl"
export DB_HOSTING_LABEL="LOCAL_ORACLE_ARM64"
mkdir -p "\$EVIDENCE_ROOT/\$RUN_ID/HOST"
{
  echo "ARM64_PLATFORM=LINUX_ARM64_ORACLE_A1"
  echo "HOST=\$(hostname)"
  uname -a
  echo "UNAME_M=\$(uname -m)"
  rustc --version
  cargo --version
  rustc -vV | awk '/host:/{print "TARGET_TRIPLE="\$2}'
  php-fpm8.3 -v 2>&1 | head -1 || php-fpm -v 2>&1 | head -1
  php -v 2>&1 | head -1
  sudo mysql -N -e 'SELECT VERSION();' || true
} | tee "\$EVIDENCE_ROOT/\$RUN_ID/HOST/identity.txt"
[[ "\$(uname -m)" == "aarch64" ]] || { echo "FAIL: not native aarch64"; exit 9; }
sha256sum \
  crates/exyonq-cfd-dataplane/src/fcgi_route.rs \
  crates/exyonq-cfd-dataplane/src/shard.rs \
  crates/exyonq-cfd-dataplane/src/fcgi_exec.rs \
  crates/exyonq-cfd-gen/src/route_table.rs \
  crates/exyonq-cfd-gen/src/composite.rs \
  crates/exyonq-cfd-control/src/project.rs \
  | tee "\$EVIDENCE_ROOT/\$RUN_ID/HOST/cfd_file_sha256.txt"
chmod +x scripts/reality/run-phase6e-symlink-same-binary-reseal-netcup.sh \
  scripts/reality/run-phase6e-generic-php-routing-fixture.sh \
  scripts/reality/run-phase6e-wordpress-qualification-netcup.sh
bash scripts/reality/run-phase6e-symlink-same-binary-reseal-netcup.sh
BIN="$REMOTE_WS/target/release/exyonq-dataplane"
if [[ -x "\$BIN" ]]; then
  file "\$BIN" | tee "\$EVIDENCE_ROOT/\$RUN_ID/HOST/binary_file.txt"
  readelf -h "\$BIN" 2>/dev/null | rg -i 'Machine|Class|Data' | tee "\$EVIDENCE_ROOT/\$RUN_ID/HOST/readelf.txt" || true
fi
REMOTE

echo "[p6f] pull evidence"
rsync -az -e "ssh $SSH_OPTS" \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase6f-dual-arch-wordpress-qualification/${RUN_ID}/" \
  "$LOCAL_EV/" || true

# Cross-arch digest compare
if [[ -f "$LOCAL_EV/HOST/cfd_file_sha256.txt" ]]; then
  if cmp -s "$LOCAL_EV/AMD64_SIDE_CFD_DIGESTS.txt" "$LOCAL_EV/HOST/cfd_file_sha256.txt"; then
    echo "CROSS_ARCH_SOURCE_EQUIVALENCE=PASS" | tee "$LOCAL_EV/CROSS_ARCH_SOURCE.txt"
  else
    echo "CROSS_ARCH_SOURCE_EQUIVALENCE=FAIL" | tee "$LOCAL_EV/CROSS_ARCH_SOURCE.txt"
    diff -u "$LOCAL_EV/AMD64_SIDE_CFD_DIGESTS.txt" "$LOCAL_EV/HOST/cfd_file_sha256.txt" || true
  fi
fi

echo "[p6f] done LOCAL_EV=$LOCAL_EV"
find "$LOCAL_EV" -maxdepth 3 -type f | head -120
