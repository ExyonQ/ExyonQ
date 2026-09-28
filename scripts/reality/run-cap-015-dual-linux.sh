#!/usr/bin/env bash
# CAPABILITY_015 WAF dual-Linux REAL PRODUCTION E2E — Netcup amd64 ∥ Oracle arm64.
# Canonical matrix: FEATURE_ID=waf
# PRODUCT_CONTRACT=Native WAF block/monitor + abuse rate_limit/challenge on real HTTP.
# Real HTTP only: curl/sockets, real exyonq binary, real upstream peer process.
# PUSH/GHCR forbidden. CAPABILITY_016 must not start.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP015_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'\"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap015-waf-$RUN_ID"
STATUS_JSON="$ROOT/.exyonq-local/status/V044-CAPABILITY-015-EVIDENCE.json"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

SEALED_LOCK="82f6e23724995ba7ec413b36732fc6c20694a1c73be5cd629176852cd522685a"
SEALED_TOOLCHAIN="1.98.1"
CAP015_HEAD_BEFORE="515c6fe7e699cb3da65472cd295e53137db80652"

PUSH=NO
GHCR_WRITE=NO
CAPABILITY_016_STARTED=NO

export PUSH GHCR_WRITE CAPABILITY_016_STARTED

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "RUST_TOOLCHAIN=$TOOLCHAIN"
echo "EVIDENCE=$EVIDENCE"
echo "STATUS_JSON=$STATUS_JSON"
echo "CAPABILITY_015=waf"
echo "CAPABILITY_016_STARTED=$CAPABILITY_016_STARTED"
echo "CAP015_HEAD_BEFORE=$CAP015_HEAD_BEFORE"
echo "CURRENT_HEAD=$HEAD"
echo "SEALED_LOCK=$SEALED_LOCK"
echo "SEALED_TOOLCHAIN=$SEALED_TOOLCHAIN"
echo "PUSH=$PUSH"
echo "GHCR_WRITE=$GHCR_WRITE"

if [[ "$LOCK_SHA" != "$SEALED_LOCK" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (Cargo.lock)"
  exit 2
fi
if [[ "$TOOLCHAIN" != "$SEALED_TOOLCHAIN" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (rust-toolchain)"
  exit 2
fi

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_015=waf
CAPABILITY_NAME=waf
FEATURE_ID=waf
PRODUCT_CONTRACT=Native WAF signature block/monitor plus abuse rate_limit/challenge
REAL_HTTP_ONLY=YES
REAL_EXYONQ_BINARY=YES
REAL_UPSTREAM_PEER_PROCESS=YES
NO_MOCKS_STUBS_SYNTHETIC_ERR=YES
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
CAP015_HEAD_BEFORE=$CAP015_HEAD_BEFORE
CAP015_CARGO_LOCK_SHA256=$LOCK_SHA
CAP015_RUST_TOOLCHAIN=$TOOLCHAIN
DEPENDENCY_BASELINE_DRIFT=NO
CAPABILITY_016_STARTED=NO
PUSH=NO
GHCR_WRITE=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD=cargo build -p exyonq -p exyonqctl --release --locked
EOF

rsync_tree() {
  local host="$1" workspace="$2"
  echo "[$(date -u +%H:%M:%S)] rsync -> ${host}:${workspace}"
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" "mkdir -p '$workspace'"
  # Root-only staging exclude: '/exyonq-waf/' must NOT match crates/exyonq-waf.
  rsync -az --delete \
    --exclude target --exclude .git \
    --exclude benchmarks/results --exclude benchmarks/results-dev \
    --exclude .exyonq-local \
    --exclude '/exyonq-waf/' \
    -e "ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30" \
    "$ROOT/" "${host}:${workspace}/"
}

run_arch() {
  local label="$1" host="$2" workspace="$3" arch="$4" target_dir="$5"
  local log="$EVIDENCE/${label}.log"
  local remote_json="/tmp/cap015-waf-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap015-waf-${RUN_ID}-${arch}-ev"
  local remote_sha="/tmp/cap015-waf-${RUN_ID}-${arch}.sha256"
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
echo "RUSTUP_TOOLCHAIN_BEFORE=$(rustup show active-toolchain 2>/dev/null || true)"
rustup toolchain install 1.98.1 --profile minimal
rustup override set 1.98.1
echo "RUSTUP_TOOLCHAIN_AFTER=$(rustup show active-toolchain 2>/dev/null || true)"
echo "BUILD_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cargo build -p exyonq -p exyonqctl --release --locked
BIN="$TD/release/exyonq"
CTL="$TD/release/exyonqctl"
test -x "$BIN"
test -x "$CTL"
echo "BUILD_END=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
BIN_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
CTL_SHA256="$(sha256sum "$CTL" | awk '{print $1}')"
echo "BIN_SHA256=$BIN_SHA256"
echo "CTL_SHA256=$CTL_SHA256"
printf 'BIN_SHA256=%s\nCTL_SHA256=%s\n' "$BIN_SHA256" "$CTL_SHA256" >"$SHA_OUT"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV" EXYONQ_BIN="$BIN" EXYONQCTL_BIN="$CTL"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-015-waf-e2e.py"
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
  scp -o BatchMode=yes -o ConnectTimeout=30 \
    "${host}:${remote_sha}" "$EVIDENCE/${label}.sha256" || true
  echo "$rc" >"$EVIDENCE/${label}.exit"
  return 0
}

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap015-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap015-target-arm64" &
pid_arm=$!

set +e
wait "$pid_amd"; rc_amd=$?
wait "$pid_arm"; rc_arm=$?
set -e
echo "wait_amd=$rc_amd wait_arm=$rc_arm"

python3 - "$EVIDENCE" "$STATUS_JSON" "$RUN_ID" "$HEAD" "$TREE" "$LOCK_SHA" "$TOOLCHAIN" "$CAP015_HEAD_BEFORE" <<'PY'
import json
import sys
from pathlib import Path

ev = Path(sys.argv[1])
status_json = Path(sys.argv[2])
run_id, head, tree, lock_sha, toolchain, cap_head_before = sys.argv[3:9]

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
amd_pass = amd.get("FINAL_RESULT") == "PASS"
arm_pass = arm.get("FINAL_RESULT") == "PASS"
closed = amd_pass and arm_pass

def both(key: str) -> dict:
    return {"amd64": amd.get(key), "arm64": arm.get(key)}

summary = {
    "CAPABILITY_ID": "015",
    "CAPABILITY_015": "waf",
    "FEATURE_ID": "waf",
    "CAPABILITY_NAME": "waf",
    "PRODUCT_CONTRACT": "Native WAF signature block/monitor plus abuse rate_limit/challenge on real HTTP paths",
    "REAL_HTTP_ONLY": "YES",
    "REAL_EXYONQ_BINARY": "YES",
    "REAL_UPSTREAM_PEER_PROCESS": "YES",
    "NO_MOCKS_STUBS_SYNTHETIC_ERR": "YES",
    "PRE_CAP015_DUAL_LINUX_WAF_PROBE": "PASS_SUPPLEMENTAL",
    "WAF_LOGIC_P3_A_FINAL_CLASSIFICATION": "FIXED",
    "WAF_LOGIC_P3_B_FINAL_CLASSIFICATION": "FIXED",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "CAP015_HEAD_BEFORE": cap_head_before,
    "CAP015_CARGO_LOCK_SHA256": lock_sha,
    "CAP015_RUST_TOOLCHAIN": toolchain,
    "DEPENDENCY_BASELINE_DRIFT": "NO",
    "SEALED_LOCK": "82f6e23724995ba7ec413b36732fc6c20694a1c73be5cd629176852cd522685a",
    "SEALED_TOOLCHAIN": "1.98.1",
    "NETCUP_AMD64_BINARY_SHA256": amd.get("BINARY_SHA256") or amd_sha.get("BIN_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("BINARY_SHA256") or arm_sha.get("BIN_SHA256"),
    "NETCUP_AMD64_EXYONQCTL_SHA256": amd.get("EXYONQCTL_BINARY_SHA256") or amd_sha.get("CTL_SHA256"),
    "ORACLE_ARM64_EXYONQCTL_SHA256": arm.get("EXYONQCTL_BINARY_SHA256") or arm_sha.get("CTL_SHA256"),
    "CAP015_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else amd.get("FINAL_RESULT", "FAIL"),
    "CAP015_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else arm.get("FINAL_RESULT", "FAIL"),
    "CAP015_FASTCGI": both("CAP015_FASTCGI"),
    "CAP015_HTTP2": both("CAP015_HTTP2"),
    "CAP015_HTTP3": both("CAP015_HTTP3"),
    "WAF_CROSS_CAPABILITY_INVARIANTS": both("WAF_CROSS_CAPABILITY_INVARIANTS"),
    "PRODUCT_DEFECT": "YES" if (amd.get("PRODUCT_DEFECT") == "YES" or arm.get("PRODUCT_DEFECT") == "YES") else "NO",
    "CAPABILITY_015_V044_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "CAPABILITY_016_STARTED": "NO",
    "PUSH": "NO",
    "GHCR_WRITE": "NO",
    "amd64": amd,
    "arm64": arm,
}
(ev / "SUMMARY.json").write_text(json.dumps(summary, indent=2) + "\n")
status_json.write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in summary if k not in ("amd64", "arm64")}, indent=2))
sys.exit(0 if closed else 1)
PY
