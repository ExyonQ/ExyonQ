#!/usr/bin/env bash
# Orchestrate R3C loadgen capacity on Netcup amd64 + Oracle arm64.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EV_DATE="${EV_DATE:-20260803}"
EV="$ROOT/.exyonq-local/tmp/r3-${EV_DATE}"
mkdir -p "$EV"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WS:-/root/exyonq-dev-soak-src}"
ORACLE_WS="${ORACLE_WS:-/home/ubuntu/exyonq-dev-soak-src}"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=20"
REMOTE_EV_BASE=".exyonq-local/tmp/r3-${EV_DATE}"

sync_host() {
  local host="$1" ws="$2"
  ssh $SSH_OPTS "$host" "mkdir -p '$ws/tools/upstream/src' '$ws/scripts/r3' '$ws/$REMOTE_EV_BASE'"
  rsync -az -e "ssh $SSH_OPTS" \
    "$ROOT/tools/upstream/Cargo.toml" "$ROOT/tools/upstream/Cargo.lock" \
    "$ROOT/tools/upstream/README.md" "$host:$ws/tools/upstream/"
  rsync -az -e "ssh $SSH_OPTS" "$ROOT/tools/upstream/src/" "$host:$ws/tools/upstream/src/"
  rsync -az -e "ssh $SSH_OPTS" "$ROOT/scripts/r3/" "$host:$ws/scripts/r3/"
}

run_r3c() {
  local host="$1" ws="$2" arch="$3" target="$4" wrk2="$5"
  ssh $SSH_OPTS "$host" "bash -lc '
    set -euo pipefail
    source ~/.cargo/env 2>/dev/null || true
    cd \"$ws\"
    export ARCH_LABEL=$arch COMPETITIVE_TARGET_RPS=$target
    export WRK2_BIN=$wrk2 ROOT=\"$ws\" R3_PRODUCT_BASELINE=f0b2d67
    export EV=\"$ws/$REMOTE_EV_BASE/${arch}-r3c\"
    chmod +x scripts/r3/r3c-loadgen-capacity-remote.sh
    bash scripts/r3/r3c-loadgen-capacity-remote.sh
  '"
}

echo "[$(date -u +%H:%M:%S)] R3C sync"
sync_host "$NETCUP_HOST" "$NETCUP_WS"
sync_host "$ORACLE_HOST" "$ORACLE_WS"

echo "[$(date -u +%H:%M:%S)] R3C amd64"
set +e
run_r3c "$NETCUP_HOST" "$NETCUP_WS" amd64 50000 /root/exyonq-bv04/tools/wrk2-44a94c17 \
  | tee "$EV/r3c-amd64-orchestrator.log"
AMD_EC=$?
set -e
echo "R3C_AMD64_EXIT=$AMD_EC" | tee "$EV/r3c-amd64-exit.txt"
rsync -az -e "ssh $SSH_OPTS" \
  "$NETCUP_HOST:$NETCUP_WS/$REMOTE_EV_BASE/amd64-r3c/" "$EV/amd64-r3c/" || true

echo "[$(date -u +%H:%M:%S)] R3C arm64"
set +e
run_r3c "$ORACLE_HOST" "$ORACLE_WS" arm64 25000 /home/ubuntu/exyonq-bv04/tools/wrk2-44a94c17 \
  | tee "$EV/r3c-arm64-orchestrator.log"
ARM_EC=$?
set -e
echo "R3C_ARM64_EXIT=$ARM_EC" | tee "$EV/r3c-arm64-exit.txt"
rsync -az -e "ssh $SSH_OPTS" \
  "$ORACLE_HOST:$ORACLE_WS/$REMOTE_EV_BASE/arm64-r3c/" "$EV/arm64-r3c/" || true

echo "R3C_AMD64_EXIT=$AMD_EC R3C_ARM64_EXIT=$ARM_EC" | tee "$EV/r3c-dual-arch.txt"
test "$AMD_EC" -eq 0 -a "$ARM_EC" -eq 0
