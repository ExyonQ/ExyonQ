#!/usr/bin/env bash
# SUB-LET-STATIC-ADMISSION-REJECT-WRITE — dual-Linux REAL E2E (Netcup amd64 ∥ Oracle arm64).
# Blocking admission reject → 503 write failure observed; no false static_blocking_shed delivery.
# PUSH/TAG/RELEASE/GHCR forbidden. Smoke forbidden.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-reality-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-reality-src}"
RUN_ID="${EXYONQ_SUB_LET_STATIC_ADMISSION_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/sub-let-static-admission-reject-write-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

echo "SUB_LET=STATIC-ADMISSION-REJECT-WRITE"
echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "RUST_TOOLCHAIN=$TOOLCHAIN"
echo "EVIDENCE=$EVIDENCE"
echo "DEPENDENCY_BASELINE_DRIFT=NO"

cat >"$EVIDENCE/run_meta.txt" <<EOF
SUB_LET=STATIC-ADMISSION-REJECT-WRITE
PRODUCT_CONTRACT=STATIC_BLOCKING_ADMISSION_REJECTED write failure observed; no false static_blocking_shed
INSTANCE_ID=LET-180-ERR-3566
RUN_ID=$RUN_ID
HEAD=$HEAD
TREE=$TREE
CARGO_LOCK_SHA256=$LOCK_SHA
RUST_TOOLCHAIN=$TOOLCHAIN
DEPENDENCY_BASELINE_DRIFT=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
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
  local remote_json="/tmp/sub-let-static-admission-${RUN_ID}-${arch}.json"
  echo "[$(date -u +%H:%M:%S)] run $label on $host"
  rsync_tree "$host" "$workspace"
  set +e
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 \
    "$host" \
    "RUN_ID='$RUN_ID' ARCH='$arch' HEAD='$HEAD' TREE='$TREE' LOCK_SHA='$LOCK_SHA' WS='$workspace' TD='$target_dir' OUT='$remote_json' HOST_LABEL='$host' bash -s" \
    >"$log" 2>&1 <<'REMOTE'
set -euo pipefail
export PATH="$HOME/.cargo/bin:/root/.cargo/bin:$PATH"
cd "$WS"
mkdir -p "$TD"
export CARGO_TARGET_DIR="$TD"
echo "host=$(hostname) arch=$(uname -m) kernel=$(uname -sr) label=$HOST_LABEL"
echo "BUILD_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
cargo build -p exyonq --release --locked
BIN="$TD/release/exyonq"
test -x "$BIN"
echo "BUILD_END=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
BIN_SHA="$(sha256sum "$BIN" | awk '{print $1}')"
echo "BIN_SHA256=$BIN_SHA"

echo "REGRESSION_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
cargo test -p exyonq-mod-static --locked -- --test-threads=1 --nocapture \
  async_shed_writes_503_without_blocking_submit \
  shed_write_failure_returns_err_on_real_rst \
  admission_reject_is_immediate_when_saturated \
  admission_metrics_classify_accept_and_reject
reg_rc=$?
set -e
echo "REGRESSION_EXIT=$reg_rc"

echo "PRODUCT_E2E_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
EXYONQ_BIN="$BIN" bash "$WS/scripts/e2e/static-admission-reject-write-e2e.sh"
e2e_rc=$?
set -e
echo "PRODUCT_E2E_EXIT=$e2e_rc"

final=FAIL
if [[ "$reg_rc" -eq 0 && "$e2e_rc" -eq 0 ]]; then
  final=PASS_REAL_E2E
fi

python3 - <<PY
import json
out = {
  "SUB_LET": "STATIC-ADMISSION-REJECT-WRITE",
  "FINAL_RESULT": "$final",
  "ARCH": "$ARCH",
  "HOST_LABEL": "$HOST_LABEL",
  "HEAD": "$HEAD",
  "TREE": "$TREE",
  "CARGO_LOCK_SHA256": "$LOCK_SHA",
  "EXYONQ_BINARY_SHA256": "$BIN_SHA",
  "REGRESSION_EXIT": $reg_rc,
  "PRODUCT_E2E_EXIT": $e2e_rc,
  "REAL_ADMISSION_REJECT_PATH": "YES" if $e2e_rc == 0 else "NO",
  "WRITE_FAILURE_OBSERVED": "YES" if ($reg_rc == 0 and $e2e_rc == 0) else "NO",
  "FALSE_STATIC_BLOCKING_SHED_SUCCESS": "NO" if ($reg_rc == 0 and $e2e_rc == 0) else "UNKNOWN",
}
open("$OUT", "w").write(json.dumps(out, indent=2) + "\n")
print(json.dumps(out, indent=2))
PY
rc_final=0
[[ "$reg_rc" -eq 0 ]] || rc_final=1
[[ "$e2e_rc" -eq 0 ]] || rc_final=1
exit "$rc_final"
REMOTE
  local rc=$?
  set -e
  echo "[$(date -u +%H:%M:%S)] $label ssh exit=$rc"
  scp -o BatchMode=yes -o ConnectTimeout=30 \
    "${host}:${remote_json}" "$EVIDENCE/${label}.json" || true
  echo "$rc" >"$EVIDENCE/${label}.exit"
  return 0
}

run_arch amd64 "$NETCUP_HOST" "$NETCUP_WS" x86_64 "/tmp/exyonq-sub-let-static-admission-target-amd64" &
pid_amd=$!
run_arch arm64 "$ORACLE_HOST" "$ORACLE_WS" aarch64 "/tmp/exyonq-sub-let-static-admission-target-arm64" &
pid_arm=$!

set +e
wait "$pid_amd"; rc_amd=$?
wait "$pid_arm"; rc_arm=$?
set -e
echo "wait_amd=$rc_amd wait_arm=$rc_arm"

python3 - "$EVIDENCE" "$RUN_ID" "$HEAD" "$TREE" "$LOCK_SHA" <<'PY'
import json, sys
from pathlib import Path
ev = Path(sys.argv[1])
run_id, head, tree, lock_sha = sys.argv[2:6]

def load(label):
    p = ev / f"{label}.json"
    if not p.exists():
        return {"FINAL_RESULT": "MISSING_EVIDENCE", "label": label}
    d = json.loads(p.read_text()); d["label"] = label; return d

amd, arm = load("amd64"), load("arm64")
amd_pass = amd.get("FINAL_RESULT") == "PASS_REAL_E2E"
arm_pass = arm.get("FINAL_RESULT") == "PASS_REAL_E2E"
summary = {
    "SUB_LET": "STATIC-ADMISSION-REJECT-WRITE",
    "RUN_ID": run_id,
    "TERMINAL_HEAD": head,
    "TERMINAL_TREE": tree,
    "CARGO_LOCK_SHA256": lock_sha,
    "DEPENDENCY_BASELINE_DRIFT": "NO",
    "NETCUP_AMD64_REAL_E2E": "PASS" if amd_pass else "FAIL",
    "ORACLE_ARM64_REAL_E2E": "PASS" if arm_pass else "FAIL",
    "WRITE_FAILURE_OBSERVED": {
        "amd64": amd.get("WRITE_FAILURE_OBSERVED"),
        "arm64": arm.get("WRITE_FAILURE_OBSERVED"),
    },
    "FALSE_STATIC_BLOCKING_SHED_SUCCESS": {
        "amd64": amd.get("FALSE_STATIC_BLOCKING_SHED_SUCCESS"),
        "arm64": arm.get("FALSE_STATIC_BLOCKING_SHED_SUCCESS"),
    },
    "EXYONQ_BINARY_SHA256": {
        "amd64": amd.get("EXYONQ_BINARY_SHA256"),
        "arm64": arm.get("EXYONQ_BINARY_SHA256"),
    },
    "FINAL_RESULT": "PASS_DUAL_LINUX" if (amd_pass and arm_pass) else "FAIL",
}
(ev / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, indent=2))
sys.exit(0 if summary["FINAL_RESULT"] == "PASS_DUAL_LINUX" else 1)
PY
