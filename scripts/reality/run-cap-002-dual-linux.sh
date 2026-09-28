#!/usr/bin/env bash
# CAPABILITY_002 http-2 dual-Linux REAL PRODUCTION E2E — Netcup amd64 ∥ Oracle arm64.
# Canonical matrix: FEATURE_ID=http-2 (NOT static-files).
# PUSH/TAG/RELEASE/GHCR forbidden. Evidence under .exyonq-local/.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP002_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap002-http2-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

SEALED_LOCK="0d5c78445354fdd3319a3af2ed4b152456f33be0ec631fd99a030fc8c37c930e"
SEALED_TOOLCHAIN="1.98.1"
SEALED_HEAD="5c5d3b85f9637d88513c8210d022f55691ba5bef"

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "RUST_TOOLCHAIN=$TOOLCHAIN"
echo "EVIDENCE=$EVIDENCE"
echo "CAPABILITY_002=http-2"
echo "STATIC_FILES_CONTRACT_APPLIED=NO"

if [[ "$LOCK_SHA" != "$SEALED_LOCK" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (Cargo.lock)"
  exit 2
fi
if [[ "$TOOLCHAIN" != "$SEALED_TOOLCHAIN" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (rust-toolchain)"
  exit 2
fi

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_002=http-2
CAPABILITY_NAME=http-2
PRODUCT_CONTRACT=h2 over TLS ALPN; multiplex streams
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
DEPENDENCY_BASELINE_SEALED_HEAD=$SEALED_HEAD
CAP002_CARGO_LOCK_SHA256=$LOCK_SHA
CAP002_RUST_TOOLCHAIN=$TOOLCHAIN
DEPENDENCY_BASELINE_DRIFT=NO
STATIC_FILES_CONTRACT_APPLIED=NO
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
  local remote_json="/tmp/cap002-http2-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap002-http2-${RUN_ID}-${arch}-ev"
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
command -v curl >/dev/null
command -v openssl >/dev/null || echo "WARN: openssl missing (ALPN check may soft-fail via curl-only)"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-002-http2-e2e.py"
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

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap002-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap002-target-arm64" &
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
    "CAPABILITY_ID": "002",
    "CAPABILITY_002": "http-2",
    "CAPABILITY_NAME": "http-2",
    "PRODUCT_CONTRACT": "h2 over TLS ALPN; multiplex streams",
    "STATIC_FILES_CONTRACT_APPLIED": "NO",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "DEPENDENCY_BASELINE_SEALED_HEAD": "5c5d3b85f9637d88513c8210d022f55691ba5bef",
    "CAP002_CARGO_LOCK_SHA256": lock_sha,
    "CAP002_RUST_TOOLCHAIN": toolchain,
    "DEPENDENCY_BASELINE_DRIFT": "NO",
    "NETCUP_AMD64_BINARY_SHA256": amd.get("EXYONQ_BINARY_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("EXYONQ_BINARY_SHA256"),
    "HTTP2_PROTOCOL_OBSERVED": {
        "amd64": amd.get("OBSERVED_HTTP_VERSION"),
        "arm64": arm.get("OBSERVED_HTTP_VERSION"),
    },
    "ALPN_OBSERVED": {
        "amd64": amd.get("ALPN_OBSERVED"),
        "arm64": arm.get("ALPN_OBSERVED"),
    },
    "CAP002_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else "FAIL",
    "CAP002_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else "FAIL",
    "HTTP2_POSITIVE_STATUS": {
        "amd64": amd.get("HTTP2_POSITIVE_STATUS"),
        "arm64": arm.get("HTTP2_POSITIVE_STATUS"),
    },
    "HTTP2_NEGATIVE_STATUS": {
        "amd64": amd.get("HTTP2_NEGATIVE_STATUS"),
        "arm64": arm.get("HTTP2_NEGATIVE_STATUS"),
    },
    "HTTP2_MULTIPLEX_STATUS": {
        "amd64": amd.get("HTTP2_MULTIPLEX_STATUS"),
        "arm64": arm.get("HTTP2_MULTIPLEX_STATUS"),
    },
    "BODY_SHA256_STATUS": {
        "amd64": amd.get("HTTP2_BODY_SHA256_STATUS"),
        "arm64": arm.get("HTTP2_BODY_SHA256_STATUS"),
    },
    "PRODUCT_DEFECT": "YES" if (amd.get("PRODUCT_DEFECT") == "YES" or arm.get("PRODUCT_DEFECT") == "YES") else "NO",
    "HARNESS_DEFECT": "YES" if (amd.get("HARNESS_DEFECT") == "YES" or arm.get("HARNESS_DEFECT") == "YES") else "NO",
    "ENVIRONMENT_BLOCKER": "YES" if (amd.get("ENVIRONMENT_BLOCKER") == "YES" or arm.get("ENVIRONMENT_BLOCKER") == "YES") else "NO",
    "CAPABILITY_002_V044_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "NEXT_CAPABILITY": "003",
    "CAPABILITY_003_STARTED": "NO",
    "amd64": amd,
    "arm64": arm,
}
(ev / "SUMMARY.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in summary if k not in ("amd64", "arm64")}, indent=2))
sys.exit(0 if closed else 1)
PY
