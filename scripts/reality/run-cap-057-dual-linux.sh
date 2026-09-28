#!/usr/bin/env bash
# CAPABILITY_057 full-page-cache — dual Linux REAL E2E, Netcup amd64 + Oracle arm64.
# No smoke, no proxy FPC fill, no publication, no dependency or lockfile changes.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP057_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap057-full-page-cache-$RUN_ID"
STATUS_JSON="$ROOT/.exyonq-local/status/V044-CAPABILITY-057-EVIDENCE.json"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/status" "$ROOT/.exyonq-local/logs"

PUSH=NO
GHCR_WRITE=NO
CAPABILITY_061_STARTED=NO
CAP056_REOPEN=NO
CAP055_REOPEN=NO
CAP054_REOPEN=NO
export PUSH GHCR_WRITE CAPABILITY_061_STARTED

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_057=full-page-cache
FEATURE_ID=full-page-cache
PRODUCT_CONTRACT=FPC generation-scoped fpc_cache; Static and FastCGI fill; Proxy NOT_SUPPORTED
USES_SMOKE=NO
ZERO_FAKE=REQUIRED
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
CARGO_LOCK_SHA256=$LOCK_SHA
RUST_TOOLCHAIN=$TOOLCHAIN
CAP056_REOPEN=NO
CAP055_REOPEN=NO
CAP054_REOPEN=NO
LA_CAP054_008_STATUS=OPEN
CAPABILITY_061_STARTED=NO
CAP057_DEPENDENCY_CHANGE_EXPECTED=NO
PUSH=NO
GHCR_WRITE=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD=cargo build --release --locked -p exyonq -p exyonqctl
EOF

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "EVIDENCE=$EVIDENCE"
echo "CAPABILITY_057=full-page-cache"
echo "CAPABILITY_061_STARTED=$CAPABILITY_061_STARTED"
echo "CAP056_REOPEN=$CAP056_REOPEN CAP055_REOPEN=$CAP055_REOPEN CAP054_REOPEN=$CAP054_REOPEN"
echo "PUSH=$PUSH GHCR_WRITE=$GHCR_WRITE"

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
  local remote_json="/tmp/cap057-full-page-cache-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap057-full-page-cache-${RUN_ID}-${arch}-ev"
  local remote_sha="/tmp/cap057-full-page-cache-${RUN_ID}-${arch}.sha256"
  echo "[$(date -u +%H:%M:%S)] run $label on $host"
  rsync_tree "$host" "$workspace"
  set +e
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 "$host" \
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
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
if [[ -n "$TOOLCHAIN" ]]; then
  rustup toolchain install "$TOOLCHAIN" --profile minimal
  rustup override set "$TOOLCHAIN"
fi
cargo build --release --locked -p exyonq -p exyonqctl
BIN="$TD/release/exyonq"
CTL="$TD/release/exyonqctl"
test -x "$BIN" && test -x "$CTL"
BIN_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
printf 'BIN_SHA256=%s\n' "$BIN_SHA256" >"$SHA_OUT"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN" EXYONQCTL_BIN="$CTL"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-057-full-page-cache-e2e.py"
rc=$?
set -e
echo "E2E_EXIT=$rc"
test -f "$OUT"
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

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap057-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap057-target-arm64" &
pid_arm=$!
set +e
wait "$pid_amd"; rc_amd=$?
wait "$pid_arm"; rc_arm=$?
set -e

python3 - "$EVIDENCE" "$STATUS_JSON" "$RUN_ID" "$HEAD" "$TREE" "$LOCK_SHA" "$TOOLCHAIN" <<'PY'
import json
import sys
from pathlib import Path

ev = Path(sys.argv[1])
status = Path(sys.argv[2])
run_id, head, tree, lock_sha, toolchain = sys.argv[3:8]

def load(label):
    path = ev / f"{label}.json"
    if not path.exists():
        return {"FINAL_RESULT": "MISSING_EVIDENCE", "label": label}
    try:
        value = json.loads(path.read_text())
    except Exception as exc:
        return {"FINAL_RESULT": "MALFORMED_EVIDENCE", "label": label, "error": str(exc)}
    value["label"] = label
    return value

amd = load("amd64")
arm = load("arm64")
amd_pass = amd.get("FINAL_RESULT") == "PASS_REAL_PRODUCTION"
arm_pass = arm.get("FINAL_RESULT") == "PASS_REAL_PRODUCTION"
closed = amd_pass and arm_pass
summary = {
    "CAPABILITY_057": "full-page-cache",
    "FEATURE_ID": "full-page-cache",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "CARGO_LOCK_SHA256": lock_sha,
    "RUST_TOOLCHAIN": toolchain,
    "ZERO_FAKE": "PASS" if closed else "FAIL",
    "CAP056_REOPEN": "NO",
    "CAP055_REOPEN": "NO",
    "CAP054_REOPEN": "NO",
    "LA_CAP054_008_STATUS": "OPEN",
    "FPC_PROXY": "NOT_SUPPORTED",
    "CAPABILITY_061_STARTED": "NO",
    "CAPABILITY_057_NETCUP_AMD64": amd.get("FINAL_RESULT", "FAIL"),
    "CAPABILITY_057_ORACLE_ARM64": arm.get("FINAL_RESULT", "FAIL"),
    "CAPABILITY_057_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "PUSH": "NO",
    "GHCR_WRITE": "NO",
    "amd64": amd,
    "arm64": arm,
}
status.parent.mkdir(parents=True, exist_ok=True)
status.write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({
    "RUN_ID": run_id,
    "CAPABILITY_057_NETCUP_AMD64": summary["CAPABILITY_057_NETCUP_AMD64"],
    "CAPABILITY_057_ORACLE_ARM64": summary["CAPABILITY_057_ORACLE_ARM64"],
    "CAPABILITY_057_FINAL_STATUS": summary["CAPABILITY_057_FINAL_STATUS"],
}, indent=2))
sys.exit(0 if closed else 1)
PY
