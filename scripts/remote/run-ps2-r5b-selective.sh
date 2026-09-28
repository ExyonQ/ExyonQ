#!/usr/bin/env bash
# PS2-R5B — Mac orchestrator → Netcup selective completion (no full r6).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS2_RUN_ID:-ps2-r5b-selective-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_LABEL="netcup-amd64-r5b-selective"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps2-results/$RESULTS_LABEL"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps2-results/remote-orchestrator-$RUN_ID"
R5_FREEZE="$ROOT/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5-remediation/artifact-freeze.json"

FINGERPRINT_LOCAL="$ROOT/scripts/remote/ps2-tree-fingerprint.sh"
MANIFEST_LOCAL="$ROOT/scripts/remote/ps2-generate-source-manifest.sh"
IDENTITY_LOCAL="$ROOT/scripts/remote/ps2-verify-remote-identity.sh"
COMPARE_LOCAL="$ROOT/scripts/remote/ps2-compare-r5-r5b-identity.sh"
HARNESS_LOCAL="$ROOT/scripts/remote/ps2-harness-precheck.sh"
SELECTIVE_LOCAL="$ROOT/docs/benchmarks/platform-split/ps2-run-r5b-selective.sh"

chmod +x "$FINGERPRINT_LOCAL" "$MANIFEST_LOCAL" "$IDENTITY_LOCAL" "$COMPARE_LOCAL" "$HARNESS_LOCAL" "$SELECTIVE_LOCAL"
chmod +x "$ROOT/docs/benchmarks/platform-split/ps2-collect-fingerprint.sh"

mkdir -p "$ORCH_DIR" "$LOCAL_DEST"

echo "PS2-R5B orchestrator run_id=$RUN_ID"
echo "PARENT_RUN=netcup-amd64-r5"
echo "RUN_PURPOSE=SELECTIVE_COMPLETION"
echo "RESULTS_LABEL=$RESULTS_LABEL"

[[ -f "$R5_FREEZE" ]] || {
  echo "ERROR: missing r5 freeze artifact" >&2
  exit 2
}

# Mac-side r5 vs r5b identity (before rsync)
bash "$COMPARE_LOCAL" "$ROOT" "$R5_FREEZE" | tee "$ORCH_DIR/r5-r5b-mac-identity.log"

rsync_tree() {
  ssh $SSH_OPTS "$NETCUP_HOST" "mkdir -p '$NETCUP_WORKSPACE'"
  rsync -az --delete \
    --exclude target --exclude .git \
    --exclude benchmarks/results --exclude benchmarks/results-dev \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5-remediation' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5b-selective' \
    -e "ssh $SSH_OPTS" \
    "$ROOT/" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/"
}

copy_aux() {
  ssh $SSH_OPTS "$NETCUP_HOST" "mkdir -p '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5-remediation'"
  scp $SSH_OPTS "$R5_FREEZE" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5-remediation/artifact-freeze.json"
  scp $SSH_OPTS "$SELECTIVE_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-run-r5b-selective.sh"
  scp $SSH_OPTS "$COMPARE_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-compare-r5-r5b-identity.sh"
  scp $SSH_OPTS "$HARNESS_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-harness-precheck.sh"
  scp $SSH_OPTS "$IDENTITY_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-verify-remote-identity.sh"
  scp $SSH_OPTS "$FINGERPRINT_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-tree-fingerprint.sh"
  scp $SSH_OPTS "$MANIFEST_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-generate-source-manifest.sh"
}

bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$ORCH_DIR/tree-fingerprint-mac.txt"
head="$(awk -F= '/^head=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac.txt")"
dirty="$(awk -F= '/^dirty_lines=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac.txt")"
status="$(awk -F= '/^status_hash=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac.txt")"
bundle_h="$(awk -F= '/^bundle_hash=/{print $2; exit}' "$ORCH_DIR/tree-fingerprint-mac.txt")"
bundle_fp="$(awk -F= '/^bundle_only_fingerprint=/{print $2; exit}' "$ORCH_DIR/tree-fingerprint-mac.txt")"

cat >"$ROOT/.ps2-sync-manifest" <<EOF
PS2_RUN_ID=$RUN_ID
PS2_PARENT_RUN=netcup-amd64-r5
PS2_RUN_PURPOSE=SELECTIVE_COMPLETION
PS2_HEAD=$head
PS2_DIRTY_LINES=$dirty
PS2_STATUS_HASH=$status
PS2_BUNDLE_HASH=$bundle_h
PS2_BUNDLE_ONLY_FINGERPRINT=$bundle_fp
PS2_PRIMARY_HOST=$NETCUP_HOST
EOF

echo "rsync → $NETCUP_HOST"
rsync_tree
copy_aux

bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$ORCH_DIR/tree-fingerprint-post-rsync.txt"
scp $SSH_OPTS "$ROOT/.ps2-sync-manifest" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/.ps2-sync-manifest"
PS2_RUN_ID="$RUN_ID" bash "$MANIFEST_LOCAL" "$ROOT" \
  "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json" \
  | tee "$ORCH_DIR/source-manifest-mac.txt"
scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json"
scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json.sha256" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json.sha256"

echo "=== remote selective run ==="
if ssh $SSH_OPTS "$NETCUP_HOST" \
  "source ~/.cargo/env 2>/dev/null; bash '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps2-run-r5b-selective.sh'" \
  >"$ORCH_DIR/netcup-r5b.log" 2>&1; then
  echo "PASS: netcup-r5b-selective"
  R5B_RC=0
else
  R5B_RC=$?
  echo "FAIL: netcup-r5b-selective rc=$R5B_RC" >&2
  tail -100 "$ORCH_DIR/netcup-r5b.log" >&2 || true
fi

pull_results() {
  mkdir -p "$LOCAL_DEST"
  rsync -az -e "ssh $SSH_OPTS" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5b-selective/" \
    "$LOCAL_DEST/"
  mkdir -p "$LOCAL_DEST/perf-contract-runs"
  rsync -az -e "ssh $SSH_OPTS" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/benchmarks/results/perf-contract-full-"'*' \
    "$LOCAL_DEST/perf-contract-runs/" 2>/dev/null || true
}

pull_results
cp "$ORCH_DIR/netcup-r5b.log" "$LOCAL_DEST/logs/orchestrator-netcup-r5b.log" 2>/dev/null || true

# Integrity compare
python3 <<PY
import hashlib, subprocess, sys
from pathlib import Path
local_root = Path("$LOCAL_DEST")
remote_count = int(subprocess.check_output(
    ["ssh", "-o", "BatchMode=yes", "$NETCUP_HOST",
     "find ${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5b-selective -type f | wc -l"],
    text=True).strip())
local_files = list(local_root.rglob("*"))
local_files = [p for p in local_files if p.is_file()]
print(f"remote_file_count={remote_count}")
print(f"local_file_count={len(local_files)}")
print(f"R5B_ARTIFACT_PULL={'PASS' if remote_count == len(local_files) and remote_count > 0 else 'FAIL'}")
PY

echo "PS2-R5B orchestrator complete rc=$R5B_RC dest=$LOCAL_DEST"
exit "$R5B_RC"
