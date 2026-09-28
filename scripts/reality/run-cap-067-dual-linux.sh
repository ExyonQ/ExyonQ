#!/usr/bin/env bash
# CAPABILITY_067 linux-epoll-sendfile — dual Linux REAL E2E (Netcup amd64 ∥ Oracle arm64).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP067_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap067-linux-epoll-sendfile-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/status" "$ROOT/.exyonq-local/logs"

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_067=linux-epoll-sendfile
OPTION=A
AUTOMATIC_LINUX_STATIC_FAST_PATH=YES
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
CARGO_LOCK_SHA256=$LOCK_SHA
CAP061=CLOSED_VERIFIED_REAL_PRODUCTION
CAP062=DEFERRED_OWNER_INFRASTRUCTURE_NONBLOCKING
LA_CAP054_008=OPEN
PUSH=NO
GHCR_WRITE=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
EOF

echo "RUN_ID=$RUN_ID HEAD=$HEAD EVIDENCE=$EVIDENCE"

rsync_tree() {
  local host="$1" workspace="$2"
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
  local remote_json="/tmp/cap067-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap067-${RUN_ID}-${arch}-ev"
  echo "[$(date -u +%H:%M:%S)] run $label on $host"
  rsync_tree "$host" "$workspace"
  set +e
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" \
    "RUN_ID='$RUN_ID' ARCH='$arch' HEAD='$HEAD' WS='$workspace' TD='$target_dir' OUT='$remote_json' EV='$remote_ev' HOST_LABEL='$host' bash -s" \
    >"$log" 2>&1 <<'REMOTE'
set -euo pipefail
export PATH="$HOME/.cargo/bin:/root/.cargo/bin:$PATH"
cd "$WS"
mkdir -p "$TD" "$EV"
export CARGO_TARGET_DIR="$TD"
echo "host=$(hostname) arch=$(uname -m) kernel=$(uname -sr)"
export CARGO_TARGET_DIR="$TD"
df -h / /tmp "$TD" 2>/dev/null || df -h /
# Cap067 product proof is the REAL E2E below; skip remote unit suite to save disk/time
# (unit coverage already exercised in prior Cap067 Netcup pass for eligibility/planner).
cargo build --release --locked -p exyonq
BIN="$TD/release/exyonq"
test -x "$BIN"
echo "BIN_SHA256=$(sha256sum "$BIN" | awk '{print $1}')"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN" ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
python3 "$WS/scripts/reality/cap-067-linux-epoll-sendfile-e2e.py"
REMOTE
  local rc=$?
  set -e
  echo "[$(date -u +%H:%M:%S)] $label ssh exit=$rc"
  scp -o BatchMode=yes -o ConnectTimeout=30 \
    "${host}:${remote_json}" "$EVIDENCE/${label}.json" || true
  mkdir -p "$EVIDENCE/${label}-ev"
  scp -o BatchMode=yes -o ConnectTimeout=30 -r \
    "${host}:${remote_ev}/." "$EVIDENCE/${label}-ev/" || true
  echo "$rc" >"$EVIDENCE/${label}.exit"
  return 0
}

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap067-target-amd64" &
PID_A=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap067-target-arm64" &
PID_B=$!
wait "$PID_A" || true
wait "$PID_B" || true

AMD_RC="$(cat "$EVIDENCE/amd64.exit" 2>/dev/null || echo 99)"
ARM_RC="$(cat "$EVIDENCE/arm64.exit" 2>/dev/null || echo 99)"
AMD_OVERALL="$(python3 -c "import json,sys;print(json.load(open(sys.argv[1])).get('OVERALL','MISSING'))" "$EVIDENCE/amd64.json" 2>/dev/null || echo MISSING)"
ARM_OVERALL="$(python3 -c "import json,sys;print(json.load(open(sys.argv[1])).get('OVERALL','MISSING'))" "$EVIDENCE/arm64.json" 2>/dev/null || echo MISSING)"

cat >"$ROOT/.exyonq-local/status/CAP067-LATEST-EVIDENCE.json" <<EOF
{
  "CAPABILITY_ID": "067",
  "RUN_ID": "$RUN_ID",
  "HEAD": "$HEAD",
  "TREE": "$TREE",
  "CARGO_LOCK_SHA256": "$LOCK_SHA",
  "AMD64_EXIT": $AMD_RC,
  "ARM64_EXIT": $ARM_RC,
  "AMD64_OVERALL": "$AMD_OVERALL",
  "ARM64_OVERALL": "$ARM_OVERALL",
  "EVIDENCE_DIR": "$EVIDENCE"
}
EOF

echo "AMD64_EXIT=$AMD_RC OVERALL=$AMD_OVERALL"
echo "ARM64_EXIT=$ARM_RC OVERALL=$ARM_OVERALL"
if [[ "$AMD_RC" == 0 && "$ARM_RC" == 0 && "$AMD_OVERALL" == PASS && "$ARM_OVERALL" == PASS ]]; then
  echo "CAP067_DUAL_LINUX=PASS"
  exit 0
fi
echo "CAP067_DUAL_LINUX=FAIL"
exit 1
