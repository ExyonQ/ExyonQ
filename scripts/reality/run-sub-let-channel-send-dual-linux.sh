#!/usr/bin/env bash
# SUB-LET-CHANNEL-SEND — dual-Linux REAL validation (Netcup amd64 ∥ Oracle arm64).
# Target: Redis Streams try_send overflow must NOT ACK-and-drop (reconcile_needed).
# PUSH/TAG/RELEASE/GHCR forbidden. Smoke forbidden.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_SUB_LET_CHANNEL_SEND_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
EVIDENCE="$ROOT/.exyonq-local/tmp/sub-let-channel-send-$RUN_ID"
mkdir -p "$EVIDENCE"

echo "SUB_LET=CHANNEL-SEND"
echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "EVIDENCE=$EVIDENCE"

rsync_tree() {
  local host="$1" workspace="$2"
  echo "[$(date -u +%H:%M:%S)] rsync -> ${host}:${workspace}"
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" "mkdir -p '$workspace'"
  rsync -az --delete \
    --exclude target --exclude .git \
    --exclude benchmarks/results --exclude benchmarks/results-dev \
    --exclude .exyonq-local \
    -e "ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30" \
    "$ROOT/" "${host}:${workspace}/"
}

run_arch() {
  local label="$1" host="$2" workspace="$3" arch="$4" target_dir="$5"
  local log="$EVIDENCE/${label}.log"
  local remote_json="/tmp/sub-let-channel-send-${RUN_ID}-${arch}.json"
  echo "[$(date -u +%H:%M:%S)] run $label on $host"
  rsync_tree "$host" "$workspace"
  set +e
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" \
    "RUN_ID='$RUN_ID' ARCH='$arch' HEAD='$HEAD' TREE='$TREE' LOCK_SHA='$LOCK_SHA' WS='$workspace' TD='$target_dir' OUT='$remote_json' HOST_LABEL='$host' bash -s" \
    >"$log" 2>&1 <<'REMOTE'
set -euo pipefail
export PATH="$HOME/.cargo/bin:/root/.cargo/bin:$PATH"
cd "$WS"
mkdir -p "$TD"
export CARGO_TARGET_DIR="$TD"
echo "host=$(hostname) arch=$(uname -m) kernel=$(uname -sr) label=$HOST_LABEL"

# Ensure Redis on 16379 (lab default for WC7B2).
if ! (docker exec exyonq-agg-hmac-redis redis-cli ping 2>/dev/null | grep -q PONG); then
  docker start exyonq-agg-hmac-redis 2>/dev/null || \
    docker run -d --name exyonq-agg-hmac-redis -p 127.0.0.1:16379:6379 redis:7.2.7-alpine
  sleep 1
fi
docker exec exyonq-agg-hmac-redis redis-cli ping | grep -q PONG
export EXYONQ_REDIS_COORD_URL=redis://127.0.0.1:16379/

echo "REGRESSION_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
cargo test -p exyonq-cache-redis --locked --test wc7b2_redis_coordination \
  redis_queue_backpressure_marks_reconcile -- --nocapture
reg_rc=$?
set -e
echo "REGRESSION_EXIT=$reg_rc"

final=FAIL
if [[ "$reg_rc" -eq 0 ]]; then
  final=PASS
fi

cat >"$OUT" <<EOF
{
  "SUB_LET": "CHANNEL-SEND",
  "HOST_LABEL": "$HOST_LABEL",
  "ARCH": "$ARCH",
  "HEAD": "$HEAD",
  "TREE": "$TREE",
  "CARGO_LOCK_SHA256": "$LOCK_SHA",
  "REGRESSION_EXIT": $reg_rc,
  "VERDICT": "$final",
  "EXYONQ_REDIS_COORD_URL": "$EXYONQ_REDIS_COORD_URL",
  "FINISHED_UTC": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
echo "VERDICT=$final"
exit $reg_rc
REMOTE
  local rc=$?
  set -e
  scp -o BatchMode=yes -o ConnectTimeout=30 \
    "${host}:${remote_json}" "$EVIDENCE/${label}.json" 2>/dev/null || true
  return "$rc"
}

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" amd64 /tmp/exyonq-channel-send-amd64-target &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" arm64 /tmp/exyonq-channel-send-arm64-target &
pid_arm=$!

set +e
wait "$pid_amd"
rc_amd=$?
wait "$pid_arm"
rc_arm=$?
set -e

echo "NETCUP_AMD64_EXIT=$rc_amd"
echo "ORACLE_ARM64_EXIT=$rc_arm"

if [[ "$rc_amd" -eq 0 && "$rc_arm" -eq 0 ]]; then
  echo "DUAL_LINUX_VERDICT=PASS_REAL_PRODUCTION"
  exit 0
fi
echo "DUAL_LINUX_VERDICT=FAIL"
exit 1
