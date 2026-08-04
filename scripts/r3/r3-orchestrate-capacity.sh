#!/usr/bin/env bash
# Mac orchestrator: sync R3 mock + run R3B on Netcup amd64 and Oracle arm64.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# Fresh evidence dir for baseline f0b2d67 — do not reuse r3-20260802 aborted output.
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
  ssh $SSH_OPTS "$host" "mkdir -p '$ws/tools/r3-p4-mock' '$ws/scripts/r3' '$ws/$REMOTE_EV_BASE'"
  rsync -az -e "ssh $SSH_OPTS" \
    "$ROOT/tools/r3-p4-mock/Cargo.toml" \
    "$ROOT/tools/r3-p4-mock/Cargo.lock" \
    "$ROOT/tools/r3-p4-mock/README.md" \
    "$host:$ws/tools/r3-p4-mock/"
  rsync -az -e "ssh $SSH_OPTS" \
    "$ROOT/tools/r3-p4-mock/src/" \
    "$host:$ws/tools/r3-p4-mock/src/"
  rsync -az -e "ssh $SSH_OPTS" \
    "$ROOT/scripts/r3/" \
    "$host:$ws/scripts/r3/"
}

run_r3b() {
  local host="$1" ws="$2" arch="$3" threshold="$4" wrk2="$5"
  ssh $SSH_OPTS "$host" "bash -lc '
    set -euo pipefail
    source ~/.cargo/env 2>/dev/null || true
    cd \"$ws\"
    export ARCH_LABEL=$arch THRESHOLD_RPS=$threshold
    export WRK2_BIN=$wrk2 ROOT=\"$ws\"
    export EV=\"$ws/$REMOTE_EV_BASE/$arch\"
    export R3_PRODUCT_BASELINE=f0b2d67
    chmod +x scripts/r3/r3b-mock-capacity-remote.sh
    bash scripts/r3/r3b-mock-capacity-remote.sh
  '"
}

echo "[$(date -u +%H:%M:%S)] sync netcup"
sync_host "$NETCUP_HOST" "$NETCUP_WS"
echo "[$(date -u +%H:%M:%S)] sync oracle"
sync_host "$ORACLE_HOST" "$ORACLE_WS"

echo "[$(date -u +%H:%M:%S)] R3B amd64 start"
set +e
run_r3b "$NETCUP_HOST" "$NETCUP_WS" amd64 50000 /root/exyonq-bv04/tools/wrk2-44a94c17 \
  >"$EV/r3b-amd64-orchestrator.log" 2>&1
AMD_EC=$?
set -e
echo "AMD64_EXIT=$AMD_EC" | tee "$EV/r3b-amd64-exit.txt"
rsync -az -e "ssh $SSH_OPTS" \
  "$NETCUP_HOST:$NETCUP_WS/$REMOTE_EV_BASE/amd64/" \
  "$EV/amd64/" || true

echo "[$(date -u +%H:%M:%S)] R3B arm64 start"
set +e
run_r3b "$ORACLE_HOST" "$ORACLE_WS" arm64 25000 /home/ubuntu/exyonq-bv04/tools/wrk2-44a94c17 \
  >"$EV/r3b-arm64-orchestrator.log" 2>&1
ARM_EC=$?
set -e
echo "ARM64_EXIT=$ARM_EC" | tee "$EV/r3b-arm64-exit.txt"
rsync -az -e "ssh $SSH_OPTS" \
  "$ORACLE_HOST:$ORACLE_WS/$REMOTE_EV_BASE/arm64/" \
  "$EV/arm64/" || true

echo "AMD64_EXIT=$AMD_EC ARM64_EXIT=$ARM_EC" | tee "$EV/r3b-dual-arch.txt"
test "$AMD_EC" -eq 0 -a "$ARM_EC" -eq 0
