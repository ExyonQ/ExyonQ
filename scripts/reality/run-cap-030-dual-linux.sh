#!/usr/bin/env bash
# CAPABILITY_030 proxy-streaming dual-Linux REAL E2E — Netcup amd64 ∥ Oracle arm64.
# ZERO_FAKE. Cap052/051/041/040/048 must not reopen.
# PUSH/GHCR forbidden.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP030_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'\"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap030-proxy-streaming-$RUN_ID"
STATUS_JSON="$ROOT/.exyonq-local/status/V044-CAPABILITY-030-EVIDENCE.json"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

PUSH=NO
GHCR_WRITE=NO
CAP063_REOPEN=NO
CAP052_REOPEN=NO
CAP051_REOPEN=NO
CAP037_REOPEN=NO
CAP039_REOPEN=NO
export PUSH GHCR_WRITE CAP052_REOPEN CAP051_REOPEN

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "EVIDENCE=$EVIDENCE"
echo "CAPABILITY_030=proxy-streaming"
echo "PUSH=$PUSH"
echo "GHCR_WRITE=$GHCR_WRITE"

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_030=proxy-streaming
FEATURE_ID=proxy-streaming
PRODUCT_CONTRACT=REAL upstream HTTP stream → ExyonQ proxy → incremental downstream body
INDEPENDENT_UNIX_CLIENT=NO
PACED_REAL_UPSTREAM=YES
USES_SMOKE=NO
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
CAP030_CARGO_LOCK_SHA256=$LOCK_SHA
CAP063_REOPEN=NO
CAP052_REOPEN=NO
CAP051_REOPEN=NO
CAP037_REOPEN=NO
CAP039_REOPEN=NO
CAP041_REOPEN=NO
CAP040_REOPEN=NO
CAP048_REOPEN=NO
PUSH=NO
GHCR_WRITE=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD=cargo build --release --locked -p exyonq
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
  local remote_json="/tmp/cap030-proxy-streaming-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap030-proxy-streaming-${RUN_ID}-${arch}-ev"
  local remote_sha="/tmp/cap030-proxy-streaming-${RUN_ID}-${arch}.sha256"
  echo "[$(date -u +%H:%M:%S)] run $label on $host"
  rsync_tree "$host" "$workspace"
  set +e
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" \
    "RUN_ID='$RUN_ID' ARCH='$arch' HEAD='$HEAD' WS='$workspace' TD='$target_dir' OUT='$remote_json' EV='$remote_ev' HOST_LABEL='$host' SHA_OUT='$remote_sha' bash -s" \
    >"$log" 2>&1 <<'REMOTE'
set -euo pipefail
export PATH="$HOME/.cargo/bin:/root/.cargo/bin:$PATH"
if [[ -f "$HOME/.cargo/env" ]]; then . "$HOME/.cargo/env"; fi
if [[ -f "/root/.cargo/env" ]]; then . "/root/.cargo/env"; fi
cd "$WS"
mkdir -p "$TD" "$EV"
export CARGO_TARGET_DIR="$TD"
echo "host=$(hostname) arch=$(uname -m) kernel=$(uname -sr) label=$HOST_LABEL"
TOOLCHAIN="$(awk -F'\"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
if [[ -n "$TOOLCHAIN" ]]; then
  rustup toolchain install "$TOOLCHAIN" --profile minimal
  rustup override set "$TOOLCHAIN"
fi
echo "BUILD_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cargo build --release --locked -p exyonq
BIN="$TD/release/exyonq"
test -x "$BIN"
echo "BUILD_END=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
BIN_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
echo "BIN_SHA256=$BIN_SHA256"
printf 'BIN_SHA256=%s\n' "$BIN_SHA256" >"$SHA_OUT"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-030-proxy-streaming-e2e.py"
rc=$?
set -e
echo "E2E_EXIT=$rc"
if [[ -f "$OUT" ]]; then echo "OUT_JSON_PRESENT=YES"; else echo "OUT_JSON_PRESENT=NO"; fi
exit "$rc"
REMOTE
  local rc=$?
  set -e
  echo "[$(date -u +%H:%M:%S)] $label ssh exit=$rc"
  scp -o BatchMode=yes -o ConnectTimeout=30 "${host}:${remote_json}" "$EVIDENCE/${label}.json" || true
  scp -o BatchMode=yes -o ConnectTimeout=30 "${host}:${remote_sha}" "$EVIDENCE/${label}.sha256" || true
  echo "$rc" >"$EVIDENCE/${label}.exit"
  return 0
}

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap030-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap030-target-arm64" &
pid_arm=$!

set +e
wait "$pid_amd"; rc_amd=$?
wait "$pid_arm"; rc_arm=$?
set -e
echo "wait_amd=$rc_amd wait_arm=$rc_arm"

python3 - "$EVIDENCE" "$STATUS_JSON" "$RUN_ID" "$HEAD" "$TREE" "$LOCK_SHA" "$TOOLCHAIN" <<'PY'
import json
import sys
from pathlib import Path

ev = Path(sys.argv[1])
status_json = Path(sys.argv[2])
run_id, head, tree, lock_sha, toolchain = sys.argv[3:8]

def load(label: str) -> dict:
    p = ev / f"{label}.json"
    if not p.exists():
        return {"FINAL_RESULT": "MISSING_EVIDENCE", "label": label}
    try:
        d = json.loads(p.read_text())
    except Exception as exc:
        return {"FINAL_RESULT": "MALFORMED_EVIDENCE", "label": label, "error": str(exc)}
    d["label"] = label
    return d

def sha(label: str) -> dict:
    p = ev / f"{label}.sha256"
    out = {}
    if p.exists():
        for line in p.read_text().splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                out[k] = v
    return out

amd = load("amd64")
arm = load("arm64")
amd_sha = sha("amd64")
arm_sha = sha("arm64")
amd_pass = amd.get("FINAL_RESULT") == "PASS_REAL_PRODUCTION"
arm_pass = arm.get("FINAL_RESULT") == "PASS_REAL_PRODUCTION"
closed = amd_pass and arm_pass
summary = {
    "CAPABILITY_ID": "063",
    "CAPABILITY_030": "proxy-streaming",
    "FEATURE_ID": "proxy-streaming",
    "CAPABILITY_NAME": "Proxy streaming",
    "PRODUCT_CONTRACT": "REAL upstream HTTP stream → ExyonQ proxy → incremental downstream body",
    "USES_SMOKE": "NO",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "CAP030_CARGO_LOCK_SHA256": lock_sha,
    "CAP030_RUST_TOOLCHAIN": toolchain,
    "NETCUP_AMD64_BINARY_SHA256": amd.get("EXYONQ_BINARY_SHA256") or amd_sha.get("BIN_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("EXYONQ_BINARY_SHA256") or arm_sha.get("BIN_SHA256"),
    "CAP030_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else amd.get("FINAL_RESULT", "FAIL"),
    "CAP030_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else arm.get("FINAL_RESULT", "FAIL"),
    "CAPABILITY_030_V044_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "CAP063_REOPEN": "NO",
    "CAP052_REOPEN": "NO",
    "CAP051_REOPEN": "NO",
    "CAP041_REOPEN": "NO",
    "CAP040_REOPEN": "NO",
    "CAP048_REOPEN": "NO",
    "CAP037_REOPEN": "NO",
    "CAP039_REOPEN": "NO",
    "PUSH": "NO",
    "GHCR_WRITE": "NO",
    "amd64": amd,
    "arm64": arm,
}
status_json.write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in [
    "RUN_ID","HEAD","CAP030_NETCUP_AMD64","CAP030_ORACLE_ARM64","CAPABILITY_030_V044_FINAL_STATUS"
]}, indent=2))
raise SystemExit(0 if closed else 1)
PY
