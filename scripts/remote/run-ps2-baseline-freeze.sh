#!/usr/bin/env bash
# PS2 — Mac orchestrator → Netcup amd64 (CANONICAL). Oracle arm64 optional.
# Mac Docker nested containers are NOT authorized for PS2 canonical baseline.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
ORACLE_WORKSPACE="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-dev-soak-src}"
RUN_ORACLE="${PS2_RUN_ORACLE:-0}"
RUN_ID="${PS2_RUN_ID:-ps2-baseline-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_DIR="${PS2_RESULTS_DIR:-$ROOT/docs/benchmarks/platform-split/ps2-results/remote-orchestrator-$RUN_ID}"

FINGERPRINT_LOCAL="$ROOT/scripts/remote/ps2-tree-fingerprint.sh"
MANIFEST_LOCAL="$ROOT/scripts/remote/ps2-generate-source-manifest.sh"
IDENTITY_LOCAL="$ROOT/scripts/remote/ps2-verify-remote-identity.sh"
VALIDATE_LOCAL="$ROOT/scripts/remote/validate-ps2-baseline.sh"
chmod +x "$FINGERPRINT_LOCAL" "$MANIFEST_LOCAL" "$IDENTITY_LOCAL" "$VALIDATE_LOCAL"
chmod +x "$ROOT/docs/benchmarks/platform-split/ps2-collect-fingerprint.sh"
chmod +x "$ROOT/docs/benchmarks/platform-split/ps2-run-baseline.sh"

mkdir -p "$RESULTS_DIR"

echo "PS2 canonical orchestrator run_id=$RUN_ID"
echo "PRIMARY_BASELINE_HOST=NETCUP_AMD64 ($NETCUP_HOST)"
echo "SECONDARY_BASELINE_HOST=$([[ "$RUN_ORACLE" == 1 ]] && echo LINUX_ARM64_ORACLE || echo NOT_RUN)"
echo "MAC_DOCKER_CANONICAL=NO"

ssh -G "$NETCUP_HOST" >/dev/null
ssh $SSH_OPTS "$NETCUP_HOST" 'printf "netcup host=%s arch=%s\n" "$(hostname)" "$(uname -m)"'

bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$RESULTS_DIR/tree-fingerprint-mac.txt"
EXPECTED_FP="$(awk -F= '/^fingerprint=/{print $2; exit}' "$RESULTS_DIR/tree-fingerprint-mac.txt")"
[[ -n "$EXPECTED_FP" ]] || exit 2

head="$(awk -F= '/^head=/{print $2}' "$RESULTS_DIR/tree-fingerprint-mac.txt")"
dirty="$(awk -F= '/^dirty_lines=/{print $2}' "$RESULTS_DIR/tree-fingerprint-mac.txt")"
status="$(awk -F= '/^status_hash=/{print $2}' "$RESULTS_DIR/tree-fingerprint-mac.txt")"
cat >"$ROOT/.ps2-sync-manifest" <<EOF
PS2_RUN_ID=$RUN_ID
PS2_HEAD=$head
PS2_DIRTY_LINES=$dirty
PS2_STATUS_HASH=$status
PS2_FINGERPRINT=$EXPECTED_FP
PS2_BUNDLE_ONLY_FINGERPRINT=$(awk -F= '/^bundle_only_fingerprint=/{print $2}' "$RESULTS_DIR/tree-fingerprint-mac.txt")
PS2_BUNDLE_HASH=$(awk -F= '/^bundle_hash=/{print $2}' "$RESULTS_DIR/tree-fingerprint-mac.txt")
PS2_PRIMARY_HOST=$NETCUP_HOST
EOF

rsync_tree() {
  local host="$1" workspace="$2"
  ssh $SSH_OPTS "$host" "mkdir -p '$workspace'"
  rsync -az --delete \
    --exclude target --exclude .git \
    --exclude benchmarks/results --exclude benchmarks/results-dev \
    --exclude 'docs/benchmarks/platform-split/ps2-results' \
    -e "ssh $SSH_OPTS" \
    "$ROOT/" "${host}:${workspace}/"
  scp $SSH_OPTS "$ROOT/.ps2-sync-manifest" "${host}:${workspace}/.ps2-sync-manifest"
}

copy_scripts() {
  local host="$1" workspace="$2"
  scp $SSH_OPTS "$VALIDATE_LOCAL" "${host}:${workspace}/scripts/remote/validate-ps2-baseline.sh"
  scp $SSH_OPTS "$FINGERPRINT_LOCAL" "${host}:${workspace}/scripts/remote/ps2-tree-fingerprint.sh"
  scp $SSH_OPTS "$MANIFEST_LOCAL" "${host}:${workspace}/scripts/remote/ps2-generate-source-manifest.sh"
  scp $SSH_OPTS "$IDENTITY_LOCAL" "${host}:${workspace}/scripts/remote/ps2-verify-remote-identity.sh"
}

run_remote() {
  local label="$1" host="$2" workspace="$3" arch="$4" log="$5"
  if ssh $SSH_OPTS "$host" \
    "source ~/.cargo/env 2>/dev/null; bash '$workspace/scripts/remote/validate-ps2-baseline.sh' --workspace '$workspace' --expected-arch '$arch' --expected-fingerprint '$EXPECTED_FP'" \
    >"$log" 2>&1; then
    echo "PASS: $label"
  else
    echo "FAIL: $label" >&2
    tail -80 "$log" >&2 || true
    exit 1
  fi
}

pull_results() {
  local host="$1" workspace="$2" label="$3"
  local dest="$ROOT/docs/benchmarks/platform-split/ps2-results/$label"
  mkdir -p "$dest"
  rsync -az -e "ssh $SSH_OPTS" \
    "${host}:${workspace}/docs/benchmarks/platform-split/ps2-results/" \
    "$dest/" || true
  mkdir -p "$dest/perf-contract-runs"
  rsync -az -e "ssh $SSH_OPTS" \
    "${host}:${workspace}/benchmarks/results/perf-contract-full-"'*' \
    "$dest/perf-contract-runs/" 2>/dev/null || true
}

echo "rsync → $NETCUP_HOST"
rsync_tree "$NETCUP_HOST" "$NETCUP_WORKSPACE"
copy_scripts "$NETCUP_HOST" "$NETCUP_WORKSPACE"
# Refresh bundle fingerprint after rsync payload is fixed (excludes mutable ps2-results).
bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$RESULTS_DIR/tree-fingerprint-post-rsync.txt"
EXPECTED_BUNDLE_FP="$(awk -F= '/^bundle_only_fingerprint=/{print $2; exit}' "$RESULTS_DIR/tree-fingerprint-post-rsync.txt")"
bundle_h="$(awk -F= '/^bundle_hash=/{print $2; exit}' "$RESULTS_DIR/tree-fingerprint-post-rsync.txt")"
[[ -n "$EXPECTED_BUNDLE_FP" ]] || exit 2
cat >"$ROOT/.ps2-sync-manifest" <<EOF
PS2_RUN_ID=$RUN_ID
PS2_HEAD=$head
PS2_DIRTY_LINES=$dirty
PS2_STATUS_HASH=$status
PS2_FINGERPRINT=$EXPECTED_FP
PS2_BUNDLE_ONLY_FINGERPRINT=$EXPECTED_BUNDLE_FP
PS2_BUNDLE_HASH=$bundle_h
PS2_PRIMARY_HOST=$NETCUP_HOST
EOF
scp $SSH_OPTS "$ROOT/.ps2-sync-manifest" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/.ps2-sync-manifest"
PS2_RUN_ID="$RUN_ID" bash "$MANIFEST_LOCAL" "$ROOT" \
  "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json" \
  | tee "$RESULTS_DIR/source-manifest-mac.txt"
MANIFEST_HASH="$(awk -F= '/^manifest_sha256=/{print $2; exit}' "$RESULTS_DIR/source-manifest-mac.txt")"
scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json"
scp $SSH_OPTS "$ROOT/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json.sha256" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json.sha256"
echo "expected_bundle_only_fingerprint=$EXPECTED_BUNDLE_FP"
echo "expected_manifest_sha256=$MANIFEST_HASH"
run_remote "netcup-amd64" "$NETCUP_HOST" "$NETCUP_WORKSPACE" x86_64 \
  "$RESULTS_DIR/netcup-amd64.log"
pull_results "$NETCUP_HOST" "$NETCUP_WORKSPACE" "netcup-amd64"

if [[ "$RUN_ORACLE" == "1" ]]; then
  ssh -G "$ORACLE_HOST" >/dev/null
  rsync_tree "$ORACLE_HOST" "$ORACLE_WORKSPACE"
  copy_scripts "$ORACLE_HOST" "$ORACLE_WORKSPACE"
  run_remote "oracle-aarch64" "$ORACLE_HOST" "$ORACLE_WORKSPACE" aarch64 \
    "$RESULTS_DIR/oracle-aarch64.log" || echo "Oracle PS2 secondary failed (non-blocking)"
  pull_results "$ORACLE_HOST" "$ORACLE_WORKSPACE" "oracle-aarch64"
fi

echo "PS2 canonical orchestrator complete: $RESULTS_DIR"
echo "fingerprint=$EXPECTED_FP"
echo "CANONICAL_BASELINE_HOST=NETCUP_AMD64"
