#!/usr/bin/env bash
# CAPABILITY_006 http-3-quic dual-Linux REAL PRODUCTION E2E — Netcup amd64 ∥ Oracle arm64.
# Canonical matrix: FEATURE_ID=http-3-quic (UDP http3_listen; GET/POST over H3; stream completion).
# Cap 007 MUST NOT start. Cap004/005 remain CLOSED. PUSH/TAG/RELEASE/GHCR forbidden.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP006_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap006-http3-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

SEALED_LOCK="0d5c78445354fdd3319a3af2ed4b152456f33be0ec631fd99a030fc8c37c930e"
SEALED_TOOLCHAIN="1.98.1"
SEALED_HEAD="5c5d3b85f9637d88513c8210d022f55691ba5bef"
EXPECTED_HEAD_BEFORE="12ec593202d714cb047d74854d429104cdbe18e2"

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "RUST_TOOLCHAIN=$TOOLCHAIN"
echo "EVIDENCE=$EVIDENCE"
echo "CAPABILITY_006=http-3-quic"
echo "CAPABILITY_007_STARTED=NO"
echo "CAP006_HEAD_BEFORE=$EXPECTED_HEAD_BEFORE"
echo "CURRENT_HEAD=$HEAD"
echo "HTTP3_PROVIDER_EXPECTED=s2n"
echo "HISTORICAL_POST_BODY_RESET_FIX=APPLICABLE_CONTEXT_ONLY"

if [[ "$LOCK_SHA" != "$SEALED_LOCK" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (Cargo.lock)"; exit 2
fi
if [[ "$TOOLCHAIN" != "$SEALED_TOOLCHAIN" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (rust-toolchain)"; exit 2
fi

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_006=http-3-quic
CAPABILITY_NAME=http-3-quic
PRODUCT_CONTRACT=UDP http3_listen; GET/POST over H3; stream completion
CONFIG_SURFACE=[http3]; server.http3_listen
HTTP3_PROVIDER_EXPECTED=s2n
HISTORICAL_POST_BODY_RESET_FIX=APPLICABLE_CONTEXT_ONLY
OPEN_DEFECT_CONTEXT=RD-008
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
CAP006_HEAD_BEFORE=$EXPECTED_HEAD_BEFORE
DEPENDENCY_BASELINE_SEALED_HEAD=$SEALED_HEAD
CAP006_CARGO_LOCK_SHA256=$LOCK_SHA
CAP006_RUST_TOOLCHAIN=$TOOLCHAIN
DEPENDENCY_BASELINE_DRIFT=NO
CAPABILITY_007_STARTED=NO
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
  local remote_json="/tmp/cap006-http3-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap006-http3-${RUN_ID}-${arch}-ev"
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
# Provider identity from strings (s2n expected; Quinn-legacy not proof)
if command -v strings >/dev/null 2>&1; then
  if strings "$BIN" | grep -Eqi 's2n.?quic|http3-provider-s2n|provider.?s2n'; then
    echo "PROVIDER_STRINGS=s2n_markers_present"
  else
    echo "PROVIDER_STRINGS=s2n_markers_absent"
  fi
fi
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-006-http3-e2e.py"
rc=$?
set -e
echo "E2E_EXIT=$rc"
if [[ -f "$OUT" ]]; then
  echo "OUT_JSON_PRESENT=YES"
else
  echo "OUT_JSON_PRESENT=NO"
fi
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

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap006-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap006-target-arm64" &
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
closed = amd_pass and arm_pass

def both(key):
    return {"amd64": amd.get(key), "arm64": arm.get(key)}

summary = {
    "CAPABILITY_ID": "006",
    "CAPABILITY_006": "http-3-quic",
    "CAPABILITY_NAME": "http-3-quic",
    "PRODUCT_CONTRACT": "UDP http3_listen; GET/POST over H3; stream completion",
    "SUPPORTED_BEHAVIOR": "s2n-default H3 GET/POST; stream-clean client completion; body integrity",
    "EXPLICIT_NON_SCOPE": [
        "Cap007+",
        "Quinn-legacy as runtime proof",
        "H1/H2 fallback as H3 PASS",
        "competitive RPS",
        "Cap004/005 beyond H3 path",
    ],
    "CAPABILITY_007_STARTED": "NO",
    "HISTORICAL_POST_BODY_RESET_FIX": "APPLICABLE_CONTEXT_ONLY",
    "HTTP3_PROVIDER_EXPECTED": "s2n",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "DEPENDENCY_BASELINE_SEALED_HEAD": "5c5d3b85f9637d88513c8210d022f55691ba5bef",
    "CAP006_CARGO_LOCK_SHA256": lock_sha,
    "CAP006_RUST_TOOLCHAIN": toolchain,
    "DEPENDENCY_BASELINE_DRIFT": "NO",
    "NETCUP_AMD64_BINARY_SHA256": amd.get("EXYONQ_BINARY_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("EXYONQ_BINARY_SHA256"),
    "CAP006_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else "FAIL",
    "CAP006_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else "FAIL",
    "HTTP3_PROVIDER_ACTUALLY_EXECUTED": both("HTTP3_PROVIDER_ACTUALLY_EXECUTED"),
    "REAL_QUIC_CONNECTION": both("REAL_QUIC_CONNECTION"),
    "REAL_HTTP3_NEGOTIATED": both("REAL_HTTP3_NEGOTIATED"),
    "OBSERVED_HTTP_VERSION": both("OBSERVED_HTTP_VERSION"),
    "HTTP3_ALPN": both("HTTP3_ALPN"),
    "CAP006_POSITIVE_STATUS": both("CAP006_POSITIVE_STATUS"),
    "CAP006_NEGATIVE_STATUS": both("CAP006_NEGATIVE_STATUS"),
    "CAP006_FAILURE_STATUS": both("CAP006_FAILURE_STATUS"),
    "CAP006_MULTIPLEX_STATUS": both("CAP006_MULTIPLEX_STATUS"),
    "HTTP3_BODY_SHA256_STATUS": both("HTTP3_BODY_SHA256_STATUS"),
    "HTTP3_CLIENT_COMPLETION_STATUS": both("HTTP3_CLIENT_COMPLETION_STATUS"),
    "TRANSPORT_RESET_COUNT_DIRECTLY_OBSERVED": both("TRANSPORT_RESET_COUNT_DIRECTLY_OBSERVED"),
    "PRODUCT_DEFECT": "YES" if (amd.get("PRODUCT_DEFECT") == "YES" or arm.get("PRODUCT_DEFECT") == "YES") else "NO",
    "CAPABILITY_006_V044_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "NEXT_CAPABILITY": "007",
    "CAPABILITY_007_STARTED": "NO",
    "amd64": amd,
    "arm64": arm,
}
(ev / "SUMMARY.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in summary if k not in ("amd64", "arm64")}, indent=2))
sys.exit(0 if closed else 1)
PY
