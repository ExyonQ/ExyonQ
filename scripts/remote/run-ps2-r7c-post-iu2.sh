#!/usr/bin/env bash
# PS2-R7C — Mac orchestrator → Netcup resumable canonical completion (post-IU2).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS2_RUN_ID:-ps2-r7c-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_LABEL="netcup-amd64-r7c-post-iu2"
LOCAL_DEST="$ROOT/docs/benchmarks/platform-split/ps2-results/$RESULTS_LABEL"
ORCH_DIR="$ROOT/docs/benchmarks/platform-split/ps2-results/remote-orchestrator-$RUN_ID"

FINGERPRINT_LOCAL="$ROOT/scripts/remote/ps2-tree-fingerprint.sh"
MANIFEST_LOCAL="$ROOT/scripts/remote/ps2-generate-source-manifest.sh"
IDENTITY_LOCAL="$ROOT/scripts/remote/ps2-verify-remote-identity.sh"
HARNESS_LOCAL="$ROOT/scripts/remote/ps2-harness-precheck.sh"
R7C_LOCAL="$ROOT/docs/benchmarks/platform-split/ps2-run-r7c-post-iu2.sh"

chmod +x "$FINGERPRINT_LOCAL" "$MANIFEST_LOCAL" "$IDENTITY_LOCAL" "$HARNESS_LOCAL" "$R7C_LOCAL"
chmod +x "$ROOT/docs/benchmarks/platform-split/ps2-collect-fingerprint.sh"
chmod +x "$ROOT/docs/benchmarks/platform-split/ps2-results/r6s-"*.py 2>/dev/null || true
chmod +x "$ROOT/docs/benchmarks/platform-split/ps2-results/r7c-"*.py 2>/dev/null || true

mkdir -p "$ORCH_DIR" "$LOCAL_DEST"

echo "PS2-R7C orchestrator run_id=$RUN_ID"
echo "RUN_CLASS=RESUMABLE_CANONICAL_COMPLETION"
echo "RESULTS_LABEL=$RESULTS_LABEL"
echo "IU2_RUN_ID=iu2-20260715T133028Z"

rsync_tree() {
  ssh $SSH_OPTS "$NETCUP_HOST" "mkdir -p '$NETCUP_WORKSPACE'"
  rsync -az --delete \
    --exclude target --exclude .git \
    --exclude benchmarks/results --exclude benchmarks/results-dev \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5-remediation' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5b-selective' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r6s-current-bundle' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r6s-current-bundle-hang-recovery' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-iu1' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-iu2' \
    --exclude 'docs/benchmarks/platform-split/ps2-results/netcup-amd64-r7c-post-iu2' \
    -e "ssh $SSH_OPTS" \
    "$ROOT/" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/"
}

copy_aux() {
  scp $SSH_OPTS "$R7C_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-run-r7c-post-iu2.sh"
  scp $SSH_OPTS "$HARNESS_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-harness-precheck.sh"
  scp $SSH_OPTS "$IDENTITY_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-verify-remote-identity.sh"
  scp $SSH_OPTS "$FINGERPRINT_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-tree-fingerprint.sh"
  scp $SSH_OPTS "$MANIFEST_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-generate-source-manifest.sh"
  scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-results/r6s-validate-scenarios.py" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/r6s-validate-scenarios.py"
  scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-results/r6s-summarize.py" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/r6s-summarize.py"
  scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-results/r7c-artifact-integrity.py" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/r7c-artifact-integrity.py" 2>/dev/null || true
  scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-results/r7c_contract_gate.py" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/r7c_contract_gate.py" 2>/dev/null || true
}

bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$ORCH_DIR/tree-fingerprint-mac-pre.txt"
head="$(awk -F= '/^head=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"
dirty="$(awk -F= '/^dirty_lines=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"
status="$(awk -F= '/^status_hash=/{print $2}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"
bundle_h="$(awk -F= '/^bundle_hash=/{print $2; exit}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"
bundle_fp="$(awk -F= '/^bundle_only_fingerprint=/{print $2; exit}' "$ORCH_DIR/tree-fingerprint-mac-pre.txt")"

cat >"$ROOT/.ps2-sync-manifest" <<EOF
PS2_RUN_ID=$RUN_ID
PS2_RUN_CLASS=RESUMABLE_CANONICAL_COMPLETION
PS2_HEAD=$head
PS2_DIRTY_LINES=$dirty
PS2_STATUS_HASH=$status
PS2_BUNDLE_HASH=$bundle_h
PS2_BUNDLE_ONLY_FINGERPRINT=$bundle_fp
PS2_PRIMARY_HOST=$NETCUP_HOST
PS2_IU2_RUN_ID=iu2-20260715T133028Z
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
    echo "R7C_SOURCE_IDENTITY=MISMATCH" | tee "$ORCH_DIR/r7c-verdicts.txt"
    exit 2
  }
  echo "R7C_SOURCE_IDENTITY=MATCH" | tee "$ORCH_DIR/r7c-verdicts-preflight.txt"
else
  echo "R7C_SOURCE_IDENTITY=MISMATCH" | tee "$ORCH_DIR/r7c-verdicts.txt"
  exit 2
fi

echo "=== remote R7C run (expect ~2-4h) ==="
R7C_RC=0
if ssh $SSH_OPTS "$NETCUP_HOST" \
  "source ~/.cargo/env 2>/dev/null; PS2_FORCE_EXYONQ_REBUILD=1 bash '$NETCUP_WORKSPACE/docs/benchmarks/platform-split/ps2-run-r7c-post-iu2.sh'" \
  >"$ORCH_DIR/netcup-r7c.log" 2>&1; then
  echo "PASS: netcup-r7c"
else
  R7C_RC=$?
  echo "R7C finished rc=$R7C_RC" >&2
  tail -120 "$ORCH_DIR/netcup-r7c.log" >&2 || true
fi

pull_results() {
  # Attempt-scoped clean destination for authority intersection (retain prior tree separately).
  CANONICAL_PULL="$LOCAL_DEST/attempt-pull/matrix-resume-${RUN_ID}"
  mkdir -p "$CANONICAL_PULL"
  rsync -az --delete -e "ssh $SSH_OPTS" \
    --exclude 'resume-attempt/' \
    --exclude 'excluded-residual-stale/' \
    --exclude 'attempt-1-checkpoint-failure/' \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r7c-post-iu2/" \
    "$CANONICAL_PULL/"
  # Also refresh working LOCAL_DEST without --delete to preserve historical residual separately.
  mkdir -p "$LOCAL_DEST"
  rsync -az -e "ssh $SSH_OPTS" \
    "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r7c-post-iu2/" \
    "$LOCAL_DEST/"
  # Pull only contract run IDs referenced by this resume (authority), not residual trees.
  mkdir -p "$CANONICAL_PULL/perf-contract-runs" "$LOCAL_DEST/perf-contract-runs"
  for rid in perf-contract-full-1784137021 perf-contract-full-1784140040; do
    rsync -az -e "ssh $SSH_OPTS" \
      "${NETCUP_HOST}:${NETCUP_WORKSPACE}/benchmarks/results/${rid}/" \
      "$CANONICAL_PULL/perf-contract-runs/${rid}/" 2>/dev/null || true
  done
  echo "CANONICAL_PULL_DIR=$CANONICAL_PULL"
}

pull_results
cp "$ORCH_DIR/netcup-r7c.log" "$LOCAL_DEST/logs/orchestrator-netcup-r7c.log" 2>/dev/null || true
cp "$ORCH_DIR/remote-identity-preflight.log" "$LOCAL_DEST/identity-gate-orchestrator.log" 2>/dev/null || true

INTEGRITY_TARGET="$LOCAL_DEST/attempt-pull/matrix-resume-${RUN_ID}"
if [[ ! -d "$INTEGRITY_TARGET" ]]; then INTEGRITY_TARGET="$LOCAL_DEST"; fi

if [[ -f "$ROOT/docs/benchmarks/platform-split/ps2-results/r7c-artifact-integrity.py" ]]; then
  python3 "$ROOT/docs/benchmarks/platform-split/ps2-results/r7c-artifact-integrity.py" \
    "$INTEGRITY_TARGET" "$NETCUP_HOST" "$NETCUP_WORKSPACE" \
    | tee "$ORCH_DIR/artifact-integrity.log"
fi

if [[ -f "$ROOT/docs/benchmarks/platform-split/ps2-results/build-canonical-index.py" ]]; then
  python3 "$ROOT/docs/benchmarks/platform-split/ps2-results/build-canonical-index.py" --r7c-only 2>/dev/null || true
fi

if [[ -f "$ROOT/docs/benchmarks/platform-split/ps2-results/r7c-finalize.py" ]]; then
  python3 "$ROOT/docs/benchmarks/platform-split/ps2-results/r7c-finalize.py" "$LOCAL_DEST" "$ORCH_DIR" "$R7C_RC" \
    | tee "$ORCH_DIR/r7c-final-verdicts.txt"
fi

echo "PS2-R7C orchestrator complete rc=$R7C_RC dest=$LOCAL_DEST"
exit "$R7C_RC"
