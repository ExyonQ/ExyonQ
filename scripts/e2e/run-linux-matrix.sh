#!/usr/bin/env bash
# Dual-arch orchestrator for authoritative e2e gates (NO-SMOKE NS5).
# Does NOT resume R3. Does NOT publish.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EV="$ROOT/.exyonq-local/tmp/no-smoke-audit-20260802/matrix"
mkdir -p "$EV"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WS:-/root/exyonq-dev-soak-src}"
ORACLE_WS="${ORACLE_WS:-/home/ubuntu/exyonq-dev-soak-src}"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=20"

# Gates that are fast enough for first matrix (OCI build is heavy — optional)
GATES=(
  static-e2e.sh
  proxy-e2e.sh
  fastcgi-php-fpm-e2e.sh
  tls-e2e.sh
  http2-e2e.sh
)
# http3 + oci optional via INCLUDE_HEAVY=1
if [[ "${INCLUDE_HEAVY:-0}" == "1" ]]; then
  GATES+=(http3-protocol-e2e.sh oci-runtime-e2e.sh)
fi

sync_host() {
  local host="$1" ws="$2"
  ssh $SSH_OPTS "$host" "mkdir -p '$ws/scripts/e2e' '$ws/scripts/smoke' '$ws/scripts/gates' '$ws/scripts/test-tls'"
  rsync -az -e "ssh $SSH_OPTS" "$ROOT/scripts/e2e/" "$host:$ws/scripts/e2e/"
  rsync -az -e "ssh $SSH_OPTS" \
    "$ROOT/scripts/smoke/kd3-proxy-smoke.sh" \
    "$ROOT/scripts/smoke/plan08-fcgi-real-smoke.sh" \
    "$ROOT/scripts/smoke/p13a-tls-reload.sh" \
    "$ROOT/scripts/smoke/p13b-http3-product-e2e.sh" \
    "$ROOT/scripts/smoke/lib-p13a-tls.sh" \
    "$host:$ws/scripts/smoke/" 2>/dev/null || true
  rsync -az -e "ssh $SSH_OPTS" "$ROOT/scripts/test-tls/" "$host:$ws/scripts/test-tls/" 2>/dev/null || true
}

run_host() {
  local host="$1" ws="$2" arch="$3"
  local out="$EV/${arch}.txt"
  ssh $SSH_OPTS "$host" "bash -lc '
    set -euo pipefail
    source ~/.cargo/env 2>/dev/null || true
    cd \"$ws\"
    mkdir -p .exyonq-local/tmp/no-smoke-audit-20260802
    chmod +x scripts/e2e/*.sh
    cargo build -q -p exyonq --bin exyonq 2>/dev/null || cargo build -p exyonq --bin exyonq
    cargo build -q -p exyonqctl --bin exyonqctl 2>/dev/null || true
    FAIL=0
    for g in ${GATES[*]}; do
      echo \"=== \$g ===\"
      if bash \"scripts/e2e/\$g\"; then echo \"GATE_PASS \$g\"; else echo \"GATE_FAIL \$g\"; FAIL=1; fi
    done
    exit \$FAIL
  '" | tee "$out"
}

echo "[ns5] sync + run amd64"
sync_host "$NETCUP_HOST" "$NETCUP_WS"
set +e
run_host "$NETCUP_HOST" "$NETCUP_WS" amd64
AMD_EC=$?
set -e
echo "AMD64_EXIT=$AMD_EC" | tee "$EV/amd64-exit.txt"

echo "[ns5] sync + run arm64"
sync_host "$ORACLE_HOST" "$ORACLE_WS"
set +e
run_host "$ORACLE_HOST" "$ORACLE_WS" arm64
ARM_EC=$?
set -e
echo "ARM64_EXIT=$ARM_EC" | tee "$EV/arm64-exit.txt"

echo "AMD64_EXIT=$AMD_EC ARM64_EXIT=$ARM_EC" | tee "$EV/summary.txt"
test "$AMD_EC" -eq 0 -a "$ARM_EC" -eq 0
