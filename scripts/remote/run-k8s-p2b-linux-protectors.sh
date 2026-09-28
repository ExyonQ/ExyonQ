#!/usr/bin/env bash
# K8S-P2B Linux dual-arch protectors (P2B-OPEN-001). Mac orchestrator.
# COMMIT/PUSH/TAG forbidden. Evidence under .exyonq-local/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${P2B_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-k8s-p2b-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-k8s-p2b-src}"
RUN_ID="${EXYONQ_P2B_PROTECTOR_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
EVIDENCE="$ROOT/.exyonq-local/release/k8s-p2b-protectors-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs"

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "EVIDENCE=$EVIDENCE"

cat >"$EVIDENCE/run_meta.txt" <<EOF
K8S_P2B_PROTECTOR_RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
FREEZE_TRACK=R5
NETCUP_HOST=$NETCUP_HOST
ORACLE_HOST=$ORACLE_HOST
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
EOF

rsync_tree() {
  local host="$1" workspace="$2"
  echo "[$(date -u +%H:%M:%S)] rsync -> ${host}:${workspace}"
  ssh $SSH_OPTS "$host" "mkdir -p '$workspace'"
  rsync -az --delete \
    --exclude target \
    --exclude .git \
    --exclude benchmarks/results \
    --exclude benchmarks/results-dev \
    --exclude .exyonq-local \
    -e "ssh $SSH_OPTS" \
    "$ROOT/" "${host}:${workspace}/"
}

run_arch() {
  local label="$1" host="$2" workspace="$3" arch="$4" target_dir="$5"
  local log="$EVIDENCE/${label}.log"
  local remote_json="/tmp/p2b-protector-${RUN_ID}-${arch}.json"
  echo "[$(date -u +%H:%M:%S)] run $label on $host"
  rsync_tree "$host" "$workspace"
  ssh $SSH_OPTS "$host" \
    "RUN_ID='$RUN_ID' ARCH='$arch' HEAD='$HEAD' WS='$workspace' TD='$target_dir' OUT='$remote_json' HOST_LABEL='$host' bash -s" <<'REMOTE' | tee "$log"
set -euo pipefail
export PATH="$HOME/.cargo/bin:/root/.cargo/bin:$PATH"
cd "$WS"
mkdir -p "$TD"
export CARGO_TARGET_DIR="$TD"
export EXYONQ_P2B_PROTECTOR_RUN_ID="$RUN_ID"
export EXYONQ_P2B_ARCH="$ARCH"
export EXYONQ_P2B_HOST="$(hostname)"
export EXYONQ_P2B_HEAD="$HEAD"
export EXYONQ_P2B_PROTECTOR_OUT="$OUT"
echo "host=$(hostname) arch=$(uname -m) kernel=$(uname -sr) nproc=$(nproc) load=$(cut -d' ' -f1-3 /proc/loadavg) label=$HOST_LABEL"
cargo test -p exyonq-mod-proxy --release --test p2b_linux_protectors -- --nocapture --test-threads=1
echo "REMOTE_JSON=$OUT"
test -f "$OUT"
REMOTE
  scp $SSH_OPTS "${host}:${remote_json}" "$EVIDENCE/${label}.json"
  echo "[$(date -u +%H:%M:%S)] fetched $EVIDENCE/${label}.json"
}

ssh $SSH_OPTS "$NETCUP_HOST" 'printf "netcup arch=%s load=%s\n" "$(uname -m)" "$(cut -d" " -f1-3 /proc/loadavg)"'
ssh $SSH_OPTS "$ORACLE_HOST" 'printf "oracle arch=%s load=%s\n" "$(uname -m)" "$(cut -d" " -f1-3 /proc/loadavg)"'

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-p2b-target-amd64"
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-p2b-target-arm64"

python3 - "$EVIDENCE" "$RUN_ID" "$HEAD" <<'PY'
import json, sys, pathlib
ev = pathlib.Path(sys.argv[1])
run_id, head = sys.argv[2], sys.argv[3]
amd = json.loads((ev/"amd64.json").read_text())
arm = json.loads((ev/"arm64.json").read_text())
amd_ok = amd.get("overall_status") == "PASS"
arm_ok = arm.get("overall_status") == "PASS"
dual = amd_ok and arm_ok
summary = {
  "K8S_P2B_PROTECTOR_RUN_ID": run_id,
  "HEAD": head,
  "AMD64_STATUS": amd.get("overall_status"),
  "ARM64_STATUS": arm.get("overall_status"),
  "K8S_P2B_PROTECTOR_STATUS": "PASS" if dual else "FAIL",
  "P2B_OPEN_001_STATUS": "CLOSED" if dual else "OPEN_REQUIRED_BEFORE_P2B_CLOSE",
  "P2B_CLOSED": dual,
  "amd64": amd,
  "arm64": arm,
}
(ev/"summary.json").write_text(json.dumps(summary, indent=2))
print(json.dumps({k: summary[k] for k in [
  "K8S_P2B_PROTECTOR_RUN_ID","K8S_P2B_PROTECTOR_STATUS","AMD64_STATUS","ARM64_STATUS",
  "P2B_OPEN_001_STATUS","P2B_CLOSED"]}, indent=2))
if not dual:
  sys.exit(2)
PY

echo "DONE evidence=$EVIDENCE"
