#!/usr/bin/env bash
# KD2 native Linux gate driver — Mac orchestrator (SSH + rsync + scp).
# No Docker, no heredocs, no sourcing remote scripts.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${KD2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
ORACLE_WORKSPACE="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-dev-soak-src}"
RUN_ID="${KD2_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_DIR="${KD2_RESULTS_DIR:-$ROOT/benchmarks/results-dev/kd2-native-gate-$RUN_ID}"
mkdir -p "$RESULTS_DIR"

VALIDATE_LOCAL="$ROOT/scripts/remote/validate-kd2-native.sh"
FINGERPRINT_LOCAL="$ROOT/scripts/remote/kd2-tree-fingerprint.sh"
chmod +x "$VALIDATE_LOCAL" "$FINGERPRINT_LOCAL"

preflight_ssh() {
  echo "--- SSH preflight ---"
  ssh -G "$NETCUP_HOST" >/dev/null
  ssh -G "$ORACLE_HOST" >/dev/null
  ssh $SSH_OPTS "$NETCUP_HOST" 'printf "host=%s arch=%s\n" "$(hostname)" "$(uname -m)"'
  ssh $SSH_OPTS "$ORACLE_HOST" 'printf "host=%s arch=%s\n" "$(hostname)" "$(uname -m)"'
}

compute_fingerprint() {
  bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$RESULTS_DIR/tree-fingerprint-mac.txt"
}

extract_fingerprint() {
  awk -F= '/^fingerprint=/{print $2; exit}' "$RESULTS_DIR/tree-fingerprint-mac.txt"
}

rsync_tree() {
  local host="$1" workspace="$2"
  echo "[$(date -u +%H:%M:%S)] rsync -> ${host}:${workspace}"
  ssh $SSH_OPTS "$host" "mkdir -p '$workspace'"
  rsync -az --delete \
    --exclude target \
    --exclude .git \
    --exclude benchmarks/results \
    --exclude benchmarks/results-dev \
    -e "ssh $SSH_OPTS" \
    "$ROOT/" "${host}:${workspace}/"
}

copy_validate_script() {
  local host="$1"
  scp $SSH_OPTS "$VALIDATE_LOCAL" "${host}:/tmp/validate-kd2-native.sh"
  scp $SSH_OPTS "$FINGERPRINT_LOCAL" "${host}:/tmp/kd2-tree-fingerprint.sh"
}

run_remote() {
  local label="$1" host="$2" workspace="$3" arch="$4" log="$5"
  local fp="$6"
  echo "[$(date -u +%H:%M:%S)] validate $label"
  ssh $SSH_OPTS "$host" \
    "bash /tmp/validate-kd2-native.sh --workspace '$workspace' --expected-arch '$arch' --expected-fingerprint '$fp'" \
    >"$log" 2>&1
  echo "PASS: $label (log: $log)"
}

echo "KD2 native gate run_id=$RUN_ID"
preflight_ssh

compute_fingerprint
EXPECTED_FP="$(extract_fingerprint)"
[[ -n "$EXPECTED_FP" ]] || {
  echo "ERROR: local fingerprint empty — aborting" >&2
  exit 2
}
echo "expected_fingerprint=$EXPECTED_FP"

# Phase A — Netcup amd64
rsync_tree "$NETCUP_HOST" "$NETCUP_WORKSPACE"
copy_validate_script "$NETCUP_HOST"
run_remote "netcup-amd64" "$NETCUP_HOST" "$NETCUP_WORKSPACE" x86_64 \
  "$RESULTS_DIR/netcup-amd64.log" "$EXPECTED_FP"

# Phase B — Oracle arm64 (only if A passed — set -e stops on failure)
rsync_tree "$ORACLE_HOST" "$ORACLE_WORKSPACE"
copy_validate_script "$ORACLE_HOST"
run_remote "oracle-aarch64" "$ORACLE_HOST" "$ORACLE_WORKSPACE" aarch64 \
  "$RESULTS_DIR/oracle-aarch64.log" "$EXPECTED_FP"

echo "KD2 native gate complete: $RESULTS_DIR"
