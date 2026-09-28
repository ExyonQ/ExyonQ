#!/usr/bin/env bash
# PS2-R6S — Mac orchestrator → Netcup selective canonical baseline (current bundle).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS2_RUN_ID:-ps2-r6s-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_LABEL="netcup-amd64-r6s-current-bundle"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps2-results/$RESULTS_LABEL"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps2-results/remote-orchestrator-$RUN_ID"

FINGERPRINT_LOCAL="$ROOT/scripts/remote/ps2-tree-fingerprint.sh"
MANIFEST_LOCAL="$ROOT/scripts/remote/ps2-generate-source-manifest.sh"
IDENTITY_LOCAL="$ROOT/scripts/remote/ps2-verify-remote-identity.sh"
HARNESS_LOCAL="$ROOT/scripts/remote/ps2-harness-precheck.sh"
R6S_LOCAL="$ROOT/docs/benchmarks/platform-split/ps2-run-r6s-current-bundle.sh"

chmod +x "$FINGERPRINT_LOCAL" "$MANIFEST_LOCAL" "$IDENTITY_LOCAL" "$HARNESS_LOCAL" "$R6S_LOCAL"
chmod +x "$ROOT/docs/benchmarks/platform-split/ps2-collect-fingerprint.sh"
chmod +x "$ROOT/docs/benchmarks/platform-split/ps2-results/r6s-"*.py 2>/dev/null || true

mkdir -p "$ORCH_DIR" "$LOCAL_DEST"

echo "PS2-R6S orchestrator run_id=$RUN_ID"
echo "RUN_CLASS=SELECTIVE_CANONICAL_RERUN"
echo "RESULTS_LABEL=$RESULTS_LABEL"
echo "R5_USE=HISTORICAL_DIAGNOSTIC_ONLY"

rsync_tree() {
  ssh $SSH_OPTS "$NETCUP_HOST" "mkdir -p '$NETCUP_WORKSPACE'"
  rsync -az --delete \
    --exclude target --exclude .git \
    --exclude benchmarks/results --exclude benchmarks/results-dev \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5-remediation' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5b-selective' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r6s-current-bundle' \
    -e "ssh $SSH_OPTS" \
    "$ROOT/" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/"
}

copy_aux() {
  scp $SSH_OPTS "$R6S_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-run-r6s-current-bundle.sh"
  scp $SSH_OPTS "$HARNESS_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-harness-precheck.sh"
  scp $SSH_OPTS "$IDENTITY_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-verify-remote-identity.sh"
  scp $SSH_OPTS "$FINGERPRINT_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-tree-fingerprint.sh"
  scp $SSH_OPTS "$MANIFEST_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-generate-source-manifest.sh"
  scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-results/r6s-validate-scenarios.py" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/r6s-validate-scenarios.py"
  scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-results/r6s-summarize.py" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/r6s-summarize.py"
}

bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$ORCH_DIR/tree-fingerprint-mac-pre.txt"
head="$(awk -F= '/^head=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"
dirty="$(awk -F= '/^dirty_lines=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"
status="$(awk -F= '/^status_hash=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"
bundle_h="$(awk -F= '/^bundle_hash=/{print $2; exit}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"
bundle_fp="$(awk -F= '/^bundle_only_fingerprint=/{print $2; exit}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"

cat >"$ROOT/.ps2-sync-manifest" <<EOF
PS2_RUN_ID=$RUN_ID
PS2_RUN_CLASS=SELECTIVE_CANONICAL_RERUN
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

bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$ORCH_DIR/tree-fingerprint-mac-post-rsync.txt"
scp $SSH_OPTS "$ROOT/.ps2-sync-manifest" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/.ps2-sync-manifest"
PS2_RUN_ID="$RUN_ID" bash "$MANIFEST_LOCAL" "$ROOT" \
  "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json" \
  | tee "$ORCH_DIR/source-manifest-mac.txt"
scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json"
scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json.sha256" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json.sha256"

echo "=== remote identity preflight ==="
if ssh $SSH_OPTS "$NETCUP_HOST" \
  "bash '$NETCUP_WORKSPACE/scripts/remote/ps2-verify-remote-identity.sh' '$NETCUP_WORKSPACE'" \
  | tee "$ORCH_DIR/remote-identity-preflight.log"; then
  grep -q 'PS2_IDENTITY_GATE=PASS' "$ORCH_DIR/remote-identity-preflight.log" || {
    echo "R6S_SOURCE_IDENTITY=MISMATCH" | tee "$ORCH_DIR/r6s-verdicts.txt"
    exit 2
  }
  echo "R6S_SOURCE_IDENTITY=MATCH" | tee "$ORCH_DIR/r6s-verdicts-preflight.txt"
else
  echo "R6S_SOURCE_IDENTITY=MISMATCH" | tee "$ORCH_DIR/r6s-verdicts.txt"
  exit 2
fi

echo "=== remote R6S run (expect ~2-3h) ==="
R6S_RC=0
if ssh $SSH_OPTS "$NETCUP_HOST" \
  "source ~/.cargo/env 2>/dev/null; bash '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps2-run-r6s-current-bundle.sh'" \
  >"$ORCH_DIR/netcup-r6s.log" 2>&1; then
  echo "PASS: netcup-r6s"
else
  R6S_RC=$?
  echo "FAIL: netcup-r6s rc=$R6S_RC" >&2
  tail -120 "$ORCH_DIR/netcup-r6s.log" >&2 || true
fi

pull_results() {
  mkdir -p "$LOCAL_DEST" "$LOCAL_DEST/perf-contract-runs"
  rsync -az -e "ssh $SSH_OPTS" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r6s-current-bundle/" \
    "$LOCAL_DEST/"
  rsync -az -e "ssh $SSH_OPTS" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/benchmarks/results/perf-contract-full-"'*' \
    "$LOCAL_DEST/perf-contract-runs/" 2>/dev/null || true
}

pull_results
cp "$ORCH_DIR/netcup-r6s.log" "$LOCAL_DEST/logs/orchestrator-netcup-r6s.log" 2>/dev/null || true
cp "$ORCH_DIR/remote-identity-preflight.log" "$LOCAL_DEST/identity-gate-orchestrator.log" 2>/dev/null || true

python3 "$ROOT/docs/benchmarks/platform-split/ps2-results/r6s-artifact-integrity.py" \
  "$LOCAL_DEST" "$NETCUP_HOST" "$NETCUP_WORKSPACE" \
  | tee "$ORCH_DIR/artifact-integrity.log"

python3 "$ROOT/docs/benchmarks/platform-split/ps2-results/build-canonical-index.py" --r6s-only
python3 "$ROOT/docs/benchmarks/platform-split/ps2-results/r6s-finalize.py" "$LOCAL_DEST" "$ORCH_DIR" "$R6S_RC" \
  | tee "$ORCH_DIR/r6s-final-verdicts.txt"

echo "PS2-R6S orchestrator complete rc=$R6S_RC dest=$LOCAL_DEST"
exit "$R6S_RC"
