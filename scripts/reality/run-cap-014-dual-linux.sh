#!/usr/bin/env bash
# CAPABILITY_014 cli-spike dual-Linux REAL PRODUCTION E2E — Netcup amd64 ∥ Oracle arm64.
# Canonical matrix: FEATURE_ID=cli-spike
# PRODUCT_CONTRACT=Fase 0 reverse-proxy spike CLI
# ENTRYPOINT=exyonq spike --listen --upstream (NOT Cap011 serve)
# Matrix DUAL_ARCH_RELEVANT=NO; campaign still executes both Linux hosts.
# Cap013 CLOSED. Cap015 MUST NOT start. PUSH/GHCR forbidden.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP014_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap014-cli-spike-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

SEALED_LOCK="0d5c78445354fdd3319a3af2ed4b152456f33be0ec631fd99a030fc8c37c930e"
SEALED_TOOLCHAIN="1.98.1"
EXPECTED_HEAD_BEFORE="15b20d2246a7b6f03233c14b3fefe74e7d037679"

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "RUST_TOOLCHAIN=$TOOLCHAIN"
echo "EVIDENCE=$EVIDENCE"
echo "CAPABILITY_014=cli-spike"
echo "CAPABILITY_015_STARTED=NO"
echo "CAP014_HEAD_BEFORE=$EXPECTED_HEAD_BEFORE"
echo "CURRENT_HEAD=$HEAD"
echo "DUAL_ARCH_MATRIX_RELEVANT=NO"

if [[ "$LOCK_SHA" != "$SEALED_LOCK" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (Cargo.lock)"; exit 2
fi
if [[ "$TOOLCHAIN" != "$SEALED_TOOLCHAIN" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (rust-toolchain)"; exit 2
fi

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_014=cli-spike
CAPABILITY_NAME=cli-spike
PRODUCT_CONTRACT=Fase 0 reverse-proxy spike CLI
CONFIG_SURFACE=CLI flags listen/upstream
ENTRYPOINT=exyonq spike --listen --upstream
CAP011_NOT_USED_AS_PROOF=YES
CAP013_NOT_USED_AS_PROOF=YES
DUAL_ARCH_MATRIX_RELEVANT=NO
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
CAP014_HEAD_BEFORE=$EXPECTED_HEAD_BEFORE
CAP014_CARGO_LOCK_SHA256=$LOCK_SHA
CAP014_RUST_TOOLCHAIN=$TOOLCHAIN
DEPENDENCY_BASELINE_DRIFT=NO
CAPABILITY_015_STARTED=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD=cargo build -p exyonq --release --locked
EOF

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
  local remote_json="/tmp/cap014-spike-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap014-spike-${RUN_ID}-${arch}-ev"
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
echo "host=$(hostname) arch=$(uname -m) kernel=$(uname -sr) label=$HOST_LABEL"
echo "BUILD_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cargo build -p exyonq --release --locked
BIN="$TD/release/exyonq"
test -x "$BIN"
echo "BUILD_END=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "BIN_SHA256=$(sha256sum "$BIN" | awk '{print $1}')"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-014-cli-spike-e2e.py"
rc=$?
set -e
echo "E2E_EXIT=$rc"
if [[ -f "$OUT" ]]; then echo "OUT_JSON_PRESENT=YES"; else echo "OUT_JSON_PRESENT=NO"; fi
exit "$rc"
REMOTE
  local rc=$?
  set -e
  echo "[$(date -u +%H:%M:%S)] $label ssh exit=$rc"
  scp -o BatchMode=yes -o ConnectTimeout=30 \
    "${host}:${remote_json}" "$EVIDENCE/${label}.json" || true
  echo "$rc" >"$EVIDENCE/${label}.exit"
  return 0
}

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap014-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap014-target-arm64" &
pid_arm=$!

set +e
wait "$pid_amd"; rc_amd=$?
wait "$pid_arm"; rc_arm=$?
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
        return {"FINAL_RESULT": "MISSING_EVIDENCE", "label": label}
    d = json.loads(p.read_text()); d["label"] = label; return d

amd, arm = load("amd64"), load("arm64")
amd_pass = amd.get("FINAL_RESULT") == "PASS_REAL_E2E"
arm_pass = arm.get("FINAL_RESULT") == "PASS_REAL_E2E"
# Matrix DUAL_ARCH_RELEVANT=NO — either arch can close; campaign requires both for Linux binary.
closed = amd_pass and arm_pass

def both(key):
    return {"amd64": amd.get(key), "arm64": arm.get(key)}

summary = {
    "CAPABILITY_ID": "014",
    "CAPABILITY_014": "cli-spike",
    "CAPABILITY_NAME": "cli-spike",
    "PRODUCT_CONTRACT": "Fase 0 reverse-proxy spike CLI",
    "SUPPORTED_BEHAVIOR": "exyonq spike --listen/--upstream; GET/POST; upstream down → 502; midbody collect Err → 502",
    "EXPLICIT_NON_SCOPE": [
        "Cap011 reverse-proxy as Cap014 proof",
        "Cap012 routing-path-proxy",
        "Cap013 reload",
        "library-only spike_proxy.rs as sole Cap014 proof",
        "platform-boundary-spike-harness",
        "competitive RPS",
        "H2/H3 upstream",
        "Cap015",
    ],
    "DUAL_ARCH_MATRIX_RELEVANT": "NO",
    "CAPABILITY_015_STARTED": "NO",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "CAP014_CARGO_LOCK_SHA256": lock_sha,
    "CAP014_RUST_TOOLCHAIN": toolchain,
    "DEPENDENCY_BASELINE_DRIFT": "NO",
    "NETCUP_AMD64_BINARY_SHA256": amd.get("EXYONQ_BINARY_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("EXYONQ_BINARY_SHA256"),
    "CAP014_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else "FAIL",
    "CAP014_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else "FAIL",
    "CAP014_L3_001_NETCUP_AMD64": (
        "PASS_REAL_PRODUCTION"
        if (amd.get("checks") or {}).get("cap014_l3_001_midbody_collect", {}).get("ok")
        else "FAIL"
    ),
    "CAP014_L3_001_ORACLE_ARM64": (
        "PASS_REAL_PRODUCTION"
        if (arm.get("checks") or {}).get("cap014_l3_001_midbody_collect", {}).get("ok")
        else "FAIL"
    ),
    "CAP014_L3_001_STATUS": both("CAP014_L3_001_STATUS"),
    "CAP014_L3_001_REAL_E2E_EXERCISED": both("CAP014_L3_001_REAL_E2E_EXERCISED"),
    "CAP014_POSITIVE_STATUS": both("CAP014_POSITIVE_STATUS"),
    "CAP014_NEGATIVE_STATUS": both("CAP014_NEGATIVE_STATUS"),
    "CAP014_FAILURE_STATUS": both("CAP014_FAILURE_STATUS"),
    "CAP014_BOUNDARY_STATUS": both("CAP014_BOUNDARY_STATUS"),
    "CAP014_CONCURRENCY_OR_LIFECYCLE_STATUS": both("CAP014_CONCURRENCY_OR_LIFECYCLE_STATUS"),
    "CAP014_CROSS_CAPABILITY_INVARIANTS": both("CAP014_CROSS_CAPABILITY_INVARIANTS"),
    "PRODUCT_DEFECT": "YES" if (amd.get("PRODUCT_DEFECT") == "YES" or arm.get("PRODUCT_DEFECT") == "YES") else "NO",
    "CAPABILITY_014_V044_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "NEXT_CAPABILITY": "015",
    "amd64": amd,
    "arm64": arm,
}
(ev / "SUMMARY.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in summary if k not in ("amd64", "arm64")}, indent=2))
sys.exit(0 if closed else 1)
PY
