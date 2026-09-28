#!/usr/bin/env bash
# CAPABILITY_061 observability-logging dual-Linux REAL E2E — Netcup amd64 ∥ Oracle arm64.
# ZERO_FAKE: real ExyonQ → real request/log event → real configured sinks. No smoke. No mocks.
# Cap061 is IN PROGRESS. Cap062/067 must not start. PUSH/GHCR forbidden.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP061_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'\"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap061-observability-logging-$RUN_ID"
STATUS_JSON="$ROOT/.exyonq-local/status/V044-CAPABILITY-061-EVIDENCE.json"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

PUSH=NO
GHCR_WRITE=NO
CAPABILITY_062_STARTED=NO
CAP067=NO
CAP054_REOPEN=NO
LA_CAP054_008=OPEN
export PUSH GHCR_WRITE CAPABILITY_062_STARTED CAP067 CAP054_REOPEN LA_CAP054_008

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "EVIDENCE=$EVIDENCE"
echo "CAPABILITY_061=observability-logging"
echo "CAPABILITY_062_STARTED=$CAPABILITY_062_STARTED"
echo "CAP067=$CAP067"
echo "CAP054_REOPEN=$CAP054_REOPEN"
echo "LA_CAP054_008=$LA_CAP054_008"
echo "PUSH=$PUSH"
echo "GHCR_WRITE=$GHCR_WRITE"

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_061=observability-logging
FEATURE_ID=observability-logging
PRODUCT_CONTRACT=REAL HTTP REQUEST → REAL EXYONQ ACCESS EVENT → REAL CONFIGURED SINK → REAL EVIDENCE
USES_SMOKE=NO
USES_MOCKS=NO
ZERO_FAKE=REQUIRED
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
CAP061_CARGO_LOCK_SHA256=$LOCK_SHA
CAP062_STARTED=NO
CAP067=NO
CAP054_REOPEN=NO
LA_CAP054_008=OPEN
PUSH=NO
GHCR_WRITE=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD=cargo build --release --locked -p exyonq -p exyonqctl
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
  local remote_json="/tmp/cap061-observability-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap061-observability-${RUN_ID}-${arch}-ev"
  local remote_sha="/tmp/cap061-observability-${RUN_ID}-${arch}.sha256"
  echo "[$(date -u +%H:%M:%S)] run $label on $host"
  rsync_tree "$host" "$workspace"
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" "rm -f '$remote_json' '$remote_sha'; rm -rf '$remote_ev'"
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
cargo build --release --locked -p exyonq -p exyonqctl
BIN="$TD/release/exyonq"
CTL="$TD/release/exyonqctl"
test -x "$BIN"
test -x "$CTL"
echo "BUILD_END=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
BIN_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
echo "BIN_SHA256=$BIN_SHA256"
printf 'BIN_SHA256=%s\n' "$BIN_SHA256" >"$SHA_OUT"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN" EXYONQCTL_BIN="$CTL"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-061-observability-logging-e2e.py"
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

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap061-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap061-target-arm64" &
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

def exit_code(label: str) -> str | None:
    p = ev / f"{label}.exit"
    if not p.exists():
        return None
    return p.read_text().strip()

def load(label: str, expected_arch: str) -> dict:
    p = ev / f"{label}.json"
    if not p.exists():
        return {"FINAL_RESULT": "MISSING_EVIDENCE", "label": label}
    try:
        d = json.loads(p.read_text())
    except Exception as exc:
        return {"FINAL_RESULT": "MALFORMED_EVIDENCE", "label": label, "error": str(exc)}
    d["label"] = label
    if d.get("HEAD") != head or d.get("ARCH_LABEL") != expected_arch:
        d["FINAL_RESULT"] = "STALE_OR_WRONG_EVIDENCE"
        d["EXPECTED_HEAD"] = head
        d["EXPECTED_ARCH_LABEL"] = expected_arch
        return d
    rc = exit_code(label)
    d["REMOTE_EXIT"] = rc
    if rc != "0":
        d["FINAL_RESULT"] = "REMOTE_EXIT_NONZERO"
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

amd = load("amd64", "x86_64")
arm = load("arm64", "aarch64")
amd_sha = sha("amd64")
arm_sha = sha("arm64")
amd_pass = amd.get("FINAL_RESULT") == "PASS_REAL_PRODUCTION"
arm_pass = arm.get("FINAL_RESULT") == "PASS_REAL_PRODUCTION"
dual_pass = amd_pass and arm_pass
summary = {
    "CAPABILITY_ID": "061",
    "FEATURE_ID": "observability-logging",
    "CAPABILITY_NAME": "Observability logging",
    "PRODUCT_CONTRACT": "REAL HTTP REQUEST → REAL EXYONQ ACCESS EVENT → REAL CONFIGURED SINK → REAL EVIDENCE",
    "USES_SMOKE": "NO",
    "USES_MOCKS": "NO",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "CAP061_CARGO_LOCK_SHA256": lock_sha,
    "CAP061_RUST_TOOLCHAIN": toolchain,
    "NETCUP_AMD64_BINARY_SHA256": amd.get("BINARY_SHA256") or amd_sha.get("BIN_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("BINARY_SHA256") or arm_sha.get("BIN_SHA256"),
    "CAP061_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else amd.get("FINAL_RESULT", "FAIL"),
    "CAP061_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else arm.get("FINAL_RESULT", "FAIL"),
    "CAP061_DUAL_LINUX_E2E": "PASS_REAL_PRODUCTION" if dual_pass else "NOT_VERIFIED",
    "CAPABILITY_061_V044_FINAL_STATUS": "NOT_CLOSED_IN_PROGRESS",
    "CAP061_STATUS": "IN_PROGRESS",
    "CAP062_STARTED": "NO",
    "CAP067": "NO",
    "CAP054_REOPEN": "NO",
    "LA_CAP054_008": "OPEN",
    "PUSH": "NO",
    "GHCR_WRITE": "NO",
    "amd64": amd,
    "arm64": arm,
}
status_json.write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({
    "RUN_ID": run_id,
    "HEAD": head,
    "CAP061_NETCUP_AMD64": summary["CAP061_NETCUP_AMD64"],
    "CAP061_ORACLE_ARM64": summary["CAP061_ORACLE_ARM64"],
    "CAP061_DUAL_LINUX_E2E": summary["CAP061_DUAL_LINUX_E2E"],
    "CAPABILITY_061_V044_FINAL_STATUS": summary["CAPABILITY_061_V044_FINAL_STATUS"],
    "CAP061_STATUS": summary["CAP061_STATUS"],
}, indent=2))
sys.exit(0 if dual_pass else 1)
PY
