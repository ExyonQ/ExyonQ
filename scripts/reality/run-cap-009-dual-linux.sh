#!/usr/bin/env bash
# CAPABILITY_009 php-fpm dual-Linux REAL PRODUCTION E2E — Netcup amd64 ∥ Oracle arm64.
# Canonical matrix: FEATURE_ID=php-fpm
# PRODUCT_CONTRACT=PHP-FPM unix/tcp via fcgi pools; product profiles php/wordpress
# Cap008 FastCGI baseline = CLOSED_VERIFIED_REAL_PRODUCTION (necessary, not sufficient).
# Cap004–008 remain CLOSED. Cap010 MUST NOT start.
# PUSH/TAG/RELEASE/GHCR forbidden.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_CAP009_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap009-phpfpm-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

SEALED_LOCK="0d5c78445354fdd3319a3af2ed4b152456f33be0ec631fd99a030fc8c37c930e"
SEALED_TOOLCHAIN="1.98.1"
SEALED_HEAD="5c5d3b85f9637d88513c8210d022f55691ba5bef"
EXPECTED_HEAD_BEFORE="655e112943979054bdf35a008c1b34c5efab4252"

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "RUST_TOOLCHAIN=$TOOLCHAIN"
echo "EVIDENCE=$EVIDENCE"
echo "CAPABILITY_009=php-fpm"
echo "CAPABILITY_010_STARTED=NO"
echo "CAP009_HEAD_BEFORE=$EXPECTED_HEAD_BEFORE"
echo "CURRENT_HEAD=$HEAD"
echo "REAL_EXTERNAL_PEER=php-fpm"
echo "CAP008_FASTCGI_BASELINE=CLOSED_VERIFIED_REAL_PRODUCTION"

if [[ "$LOCK_SHA" != "$SEALED_LOCK" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (Cargo.lock)"; exit 2
fi
if [[ "$TOOLCHAIN" != "$SEALED_TOOLCHAIN" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (rust-toolchain)"; exit 2
fi
if [[ "$HEAD" != "$EXPECTED_HEAD_BEFORE" ]]; then
  echo "HEAD_MISMATCH: expected CAP009_HEAD_BEFORE=$EXPECTED_HEAD_BEFORE got $HEAD"
  echo "Proceeding only if this is intentional harness work on same product tree."
fi

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_009=php-fpm
CAPABILITY_NAME=php-fpm
PRODUCT_CONTRACT=PHP-FPM unix/tcp via fcgi pools; product profiles php/wordpress
CONFIG_SURFACE=fcgi_pool.address/transport; product profile
REAL_EXTERNAL_PEER=php-fpm
CAP008_FASTCGI_BASELINE=CLOSED_VERIFIED_REAL_PRODUCTION
OPEN_DEFECT_CONTEXT=RD-001
RUN_ID=$RUN_ID
BASE_HEAD=$HEAD
BASE_TREE=$TREE
CAP009_HEAD_BEFORE=$EXPECTED_HEAD_BEFORE
DEPENDENCY_BASELINE_SEALED_HEAD=$SEALED_HEAD
CAP009_CARGO_LOCK_SHA256=$LOCK_SHA
CAP009_RUST_TOOLCHAIN=$TOOLCHAIN
DEPENDENCY_BASELINE_DRIFT=NO
CAPABILITY_010_STARTED=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
BUILD=cargo build -p exyonq -p exyonqctl --release --locked
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
  local remote_json="/tmp/cap009-phpfpm-${RUN_ID}-${arch}.json"
  local remote_ev="/tmp/cap009-phpfpm-${RUN_ID}-${arch}-ev"
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
PHP_FPM_BIN=""
for c in php-fpm php-fpm8.3 php-fpm8.2 php-fpm8.1; do
  if command -v "$c" >/dev/null 2>&1; then PHP_FPM_BIN="$(command -v "$c")"; break; fi
done
if [[ -z "$PHP_FPM_BIN" && -x /usr/sbin/php-fpm8.3 ]]; then PHP_FPM_BIN=/usr/sbin/php-fpm8.3; fi
if [[ -z "$PHP_FPM_BIN" ]]; then
  echo "ENVIRONMENT_BLOCKER=missing php-fpm"
  exit 3
fi
export PHP_FPM_BIN
echo "PHP_FPM_BIN=$PHP_FPM_BIN"
echo "BUILD_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cargo build -p exyonq -p exyonqctl --release --locked
BIN="$TD/release/exyonq"
CTL="$TD/release/exyonqctl"
test -x "$BIN"
test -x "$CTL"
echo "BUILD_END=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo "BIN_SHA256=$(sha256sum "$BIN" | awk '{print $1}')"
echo "CTL_SHA256=$(sha256sum "$CTL" | awk '{print $1}')"
export WS HEAD OUT_JSON="$OUT" EV_DIR="$EV"
export EXYONQ_BIN="$BIN" EXYONQCTL_BIN="$CTL"
export ARCH_LABEL="$ARCH" HOST_LABEL="$HOST_LABEL"
set +e
python3 "$WS/scripts/reality/cap-009-php-fpm-e2e.py"
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

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-cap009-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-cap009-target-arm64" &
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
    "CAPABILITY_ID": "009",
    "CAPABILITY_009": "php-fpm",
    "CAPABILITY_NAME": "php-fpm",
    "PRODUCT_CONTRACT": "PHP-FPM unix/tcp via fcgi pools; product profiles php/wordpress",
    "SUPPORTED_BEHAVIOR": "php+wordpress product profiles; unix+tcp php-fpm; SCRIPT_FILENAME/DOCUMENT_ROOT/PATH_INFO; Cap008 containment preserved",
    "EXPLICIT_NON_SCOPE": [
        "Cap008 generic FastCGI-only close as Cap009 proof",
        "Cap010 oci-container",
        "Cap007 directory-index primary",
        "Cap006 HTTP/3",
        "competitive RPS",
        "historical plan08 closer as Cap009 proof",
    ],
    "CAP008_FASTCGI_BASELINE": "CLOSED_VERIFIED_REAL_PRODUCTION",
    "CAPABILITY_010_STARTED": "NO",
    "OPEN_DEFECT_CONTEXT": "RD-001",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "DEPENDENCY_BASELINE_SEALED_HEAD": "5c5d3b85f9637d88513c8210d022f55691ba5bef",
    "CAP009_CARGO_LOCK_SHA256": lock_sha,
    "CAP009_RUST_TOOLCHAIN": toolchain,
    "DEPENDENCY_BASELINE_DRIFT": "NO",
    "NETCUP_AMD64_BINARY_SHA256": amd.get("EXYONQ_BINARY_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("EXYONQ_BINARY_SHA256"),
    "CAP009_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else "FAIL",
    "CAP009_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else "FAIL",
    "CAP009_POSITIVE_STATUS": both("CAP009_POSITIVE_STATUS"),
    "CAP009_NEGATIVE_STATUS": both("CAP009_NEGATIVE_STATUS"),
    "CAP009_FAILURE_STATUS": both("CAP009_FAILURE_STATUS"),
    "CAP009_BOUNDARY_STATUS": both("CAP009_BOUNDARY_STATUS"),
    "CAP009_CONCURRENCY_OR_LIFECYCLE_STATUS": both("CAP009_CONCURRENCY_OR_LIFECYCLE_STATUS"),
    "CAP009_CROSS_CAPABILITY_INVARIANTS": both("CAP009_CROSS_CAPABILITY_INVARIANTS"),
    "BODY_SHA256_STATUS": both("BODY_SHA256_STATUS"),
    "PRODUCT_DEFECT": "YES" if (amd.get("PRODUCT_DEFECT") == "YES" or arm.get("PRODUCT_DEFECT") == "YES") else "NO",
    "CAPABILITY_009_V044_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "NEXT_CAPABILITY": "010",
    "CAPABILITY_010_STARTED": "NO",
    "amd64": amd,
    "arm64": arm,
}
(ev / "SUMMARY.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in summary if k not in ("amd64", "arm64")}, indent=2))
sys.exit(0 if closed else 1)
PY
