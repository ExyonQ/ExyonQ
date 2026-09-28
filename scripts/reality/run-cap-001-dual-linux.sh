#!/usr/bin/env bash
# CAPABILITY_001 http-1-1 dual-Linux REAL PRODUCTION E2E — Netcup amd64 ∥ Oracle arm64.
# Requires: release/v0.4.4-integration @ sealed dependency baseline (or product-only HEAD after seal).
# PUSH/TAG/RELEASE/GHCR forbidden. Evidence under .exyonq-local/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP001_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap001-http11-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

SEALED_LOCK="0d5c78445354fdd3319a3af2ed4b152456f33be0ec631fd99a030fc8c37c930e"
SEALED_TOOLCHAIN="1.98.1"

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "RUST_TOOLCHAIN=$TOOLCHAIN"
echo "EVIDENCE=$EVIDENCE"

if [[ "$LOCK_SHA" != "$SEALED_LOCK" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (Cargo.lock)"
  echo "EXPECTED=$SEALED_LOCK"
  echo "ACTUAL=$LOCK_SHA"
  exit 2
fi
if [[ "$TOOLCHAIN" != "$SEALED_TOOLCHAIN" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (rust-toolchain)"
  exit 2
fi

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_001=http-1-1
CAPABILITY_NAME=http-1-1
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
DEPENDENCY_BASELINE_SEALED_HEAD=5c5d3b85f9637d88513c8210d022f55691ba5bef
CAP001_CARGO_LOCK_SHA256=$LOCK_SHA
CAP001_RUST_TOOLCHAIN=$TOOLCHAIN
DEPENDENCY_BASELINE_DRIFT=NO
NETCUP_HOST=$NETCUP_HOST
ORACLE_HOST=$ORACLE_HOST
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
DUAL_LINUX_ARCH_REQUIRED=YES
BUILD=cargo build -p exyonq --release --locked
EOF

rsync_tree() {
  local host="$1" workspace="$2"
  echo "[$(date -u +%H:%M:%S)] rsync -> ${host}:${workspace}"
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" "mkdir -p '$workspace'"
  rsync -az --delete \
    --exclude target \
    --exclude .git \
    --exclude benchmarks/results \
    --exclude benchmarks/results-dev \
    --exclude .exyonq-local \
    -e "ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30" \
    "$ROOT/" "${host}:${workspace}/"
}

run_arch() {
  local label="$1" host="$2" workspace="$3" arch="$4" target_dir="$5"
  local log="$EVIDENCE/${label}.log"
  local remote_json="/tmp/cap001-http11-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap001-http11-${RUN_ID}-${arch}-ev"
  echo "[$(date -u +%H:%M:%S)] run $label on $host (background build+e2e)"
  rsync_tree "$host" "$workspace"
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" \
    "RUN_ID='$RUN_ID' ARCH='$arch' HEAD='$HEAD' WS='$workspace' TD='$target_dir' OUT='$remote_json' EV='$remote_ev' HOST_LABEL='$host' LABEL='$label' bash -s" \
    >"$log" 2>&1 <<'REMOTE'
set -euo pipefail
export PATH="$HOME/.cargo/bin:/root/.cargo/bin:$PATH"
cd "$WS"
mkdir -p "$TD" "$EV"
export CARGO_TARGET_DIR="$TD"
echo "host=$(hostname) arch=$(uname -m) kernel=$(uname -sr) nproc=$(nproc) load=$(cut -d' ' -f1-3 /proc/loadavg) label=$HOST_LABEL"
echo "BUILD_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cargo build -p exyonq --release --locked
BIN="$TD/release/exyonq"
test -x "$BIN"
echo "BUILD_END=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "BIN_SHA256=$(sha256sum "$BIN" | awk '{print $1}')"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-001-http11-e2e.py"
rc=$?
set -e
echo "E2E_EXIT=$rc"
test -f "$OUT"
echo "REMOTE_JSON=$OUT"
exit "$rc"
REMOTE
  local rc=$?
  echo "[$(date -u +%H:%M:%S)] $label ssh exit=$rc"
  scp -o BatchMode=yes -o ConnectTimeout=30 \
    "${host}:${remote_json}" "$EVIDENCE/${label}.json" || true
  echo "$rc" >"$EVIDENCE/${label}.exit"
  return 0
}

# Parallel: Netcup amd64 + Oracle arm64
run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap001-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap001-target-arm64" &
pid_arm=$!

set +e
wait "$pid_amd"
rc_amd=$?
wait "$pid_arm"
rc_arm=$?
set -e

echo "wait_amd=$rc_amd wait_arm=$rc_arm"

python3 - "$EVIDENCE" "$RUN_ID" "$HEAD" "$TREE" "$LOCK_SHA" "$TOOLCHAIN" <<'PY'
import json, sys
from pathlib import Path
ev = Path(sys.argv[1])
run_id, head, tree, lock_sha, toolchain = sys.argv[2:7]

def load(label):
    p = ev / f"{label}.json"
    if not p.exists():
        return {"FINAL_RESULT": "MISSING_EVIDENCE", "PRODUCT_DEFECT": "UNKNOWN",
                "HARNESS_DEFECT": "UNKNOWN", "ENVIRONMENT_BLOCKER": "YES", "label": label}
    d = json.loads(p.read_text())
    d["label"] = label
    return d

amd = load("amd64")
arm = load("arm64")
amd_pass = amd.get("FINAL_RESULT") == "PASS_REAL_E2E"
arm_pass = arm.get("FINAL_RESULT") == "PASS_REAL_E2E"
closed = amd_pass and arm_pass
summary = {
    "CAPABILITY_ID": "001",
    "CAPABILITY_001": "http-1-1",
    "CAPABILITY_NAME": "http-1-1",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "DEPENDENCY_BASELINE_SEALED_HEAD": "5c5d3b85f9637d88513c8210d022f55691ba5bef",
    "CAP001_CARGO_LOCK_SHA256": lock_sha,
    "CAP001_RUST_TOOLCHAIN": toolchain,
    "DEPENDENCY_BASELINE_DRIFT": "NO",
    "NETCUP_AMD64_BINARY_SHA256": amd.get("EXYONQ_BINARY_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("EXYONQ_BINARY_SHA256"),
    "HTTP11_PROTOCOL_OBSERVED": {
        "amd64": amd.get("OBSERVED_HTTP_VERSION"),
        "arm64": arm.get("OBSERVED_HTTP_VERSION"),
    },
    "CAP001_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else "FAIL",
    "CAP001_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else "FAIL",
    "HTTP11_NETCUP_AMD64_REAL_E2E": "PASS" if amd_pass else "FAIL",
    "HTTP11_ORACLE_ARM64_REAL_E2E": "PASS" if arm_pass else "FAIL",
    "HTTP11_POSITIVE_STATUS": {
        "amd64": amd.get("HTTP11_POSITIVE_STATUS"),
        "arm64": arm.get("HTTP11_POSITIVE_STATUS"),
    },
    "HTTP11_NEGATIVE_STATUS": {
        "amd64": amd.get("HTTP11_NEGATIVE_STATUS"),
        "arm64": arm.get("HTTP11_NEGATIVE_STATUS"),
    },
    "HTTP11_KEEPALIVE_STATUS": {
        "amd64": amd.get("HTTP11_KEEPALIVE_STATUS"),
        "arm64": arm.get("HTTP11_KEEPALIVE_STATUS"),
    },
    "HTTP11_CONCURRENCY_STATUS": {
        "amd64": amd.get("HTTP11_CONCURRENCY_STATUS"),
        "arm64": arm.get("HTTP11_CONCURRENCY_STATUS"),
    },
    "BODY_SHA256_STATUS": {
        "amd64": amd.get("BODY_SHA256_STATUS"),
        "arm64": arm.get("BODY_SHA256_STATUS"),
    },
    "PRODUCT_DEFECT": "YES" if (amd.get("PRODUCT_DEFECT") == "YES" or arm.get("PRODUCT_DEFECT") == "YES") else "NO",
    "HARNESS_DEFECT": "YES" if (amd.get("HARNESS_DEFECT") == "YES" or arm.get("HARNESS_DEFECT") == "YES") else "NO",
    "ENVIRONMENT_BLOCKER": "YES" if (amd.get("ENVIRONMENT_BLOCKER") == "YES" or arm.get("ENVIRONMENT_BLOCKER") == "YES") else "NO",
    "CAPABILITY_001_V044_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "CAPABILITY_001_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "NEXT_CAPABILITY": "002",
    "CAPABILITY_002_STARTED": "NO",
    "amd64": amd,
    "arm64": arm,
}
(ev / "SUMMARY.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in summary if k not in ("amd64", "arm64")}, indent=2))
sys.exit(0 if closed else 1)
PY
