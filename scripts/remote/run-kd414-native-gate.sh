#!/usr/bin/env bash
# KD4.14 native Linux gate — Mac orchestrator (SSH + rsync, stop-on-fail).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${KD414_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
ORACLE_WORKSPACE="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-dev-soak-src}"
RUN_ID="${KD414_RUN_ID:-kd414-native-gate-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_DIR="${KD414_RESULTS_DIR:-$ROOT/benchmarks/results-dev/kd414-native-gate-$RUN_ID}"
RUN_ORACLE="${KD414_RUN_ORACLE:-1}"

mkdir -p "$RESULTS_DIR"

VALIDATE_LOCAL="$ROOT/scripts/remote/validate-kd414-native.sh"
FINGERPRINT_LOCAL="$ROOT/scripts/remote/kd414-tree-fingerprint.sh"
chmod +x "$VALIDATE_LOCAL" "$FINGERPRINT_LOCAL"

preflight_ssh() {
  ssh -G "$NETCUP_HOST" >/dev/null
  ssh $SSH_OPTS "$NETCUP_HOST" 'printf "host=%s arch=%s\n" "$(hostname)" "$(uname -m)"'
  if [[ "$RUN_ORACLE" == "1" ]]; then
    ssh -G "$ORACLE_HOST" >/dev/null
    ssh $SSH_OPTS "$ORACLE_HOST" 'printf "host=%s arch=%s\n" "$(hostname)" "$(uname -m)"'
  fi
}

compute_fingerprint() {
  bash "$FINGERPRINT_LOCAL" "$ROOT" | tee "$RESULTS_DIR/tree-fingerprint-mac.txt"
}

write_sync_manifest() {
  local manifest="$ROOT/.kd414-sync-manifest"
  head="$(awk -F= '/^head=/{print $2}' "$RESULTS_DIR/tree-fingerprint-mac.txt")"
  dirty="$(awk -F= '/^dirty_lines=/{print $2}' "$RESULTS_DIR/tree-fingerprint-mac.txt")"
  status="$(awk -F= '/^status_hash=/{print $2}' "$RESULTS_DIR/tree-fingerprint-mac.txt")"
  cat >"$manifest" <<EOF
KD414_HEAD=$head
KD414_DIRTY_LINES=$dirty
KD414_STATUS_HASH=$status
EOF
}

extract_fingerprint() {
  awk -F= '/^fingerprint=/{print $2; exit}' "$RESULTS_DIR/tree-fingerprint-mac.txt"
}

rsync_tree() {
  local host="$1" workspace="$2"
  ssh $SSH_OPTS "$host" "mkdir -p '$workspace'"
  rsync -az --delete \
    --exclude target --exclude .git \
    --exclude benchmarks/results --exclude benchmarks/results-dev \
    -e "ssh $SSH_OPTS" \
    "$ROOT/" "${host}:${workspace}/"
}

copy_scripts() {
  local host="$1" workspace="$2"
  scp $SSH_OPTS "$VALIDATE_LOCAL" "${host}:${workspace}/scripts/remote/validate-kd414-native.sh"
  scp $SSH_OPTS "$FINGERPRINT_LOCAL" "${host}:${workspace}/scripts/remote/kd414-tree-fingerprint.sh"
}

run_remote() {
  local label="$1" host="$2" workspace="$3" arch="$4" log="$5" fp="$6"
  if ssh $SSH_OPTS "$host" \
    "KD414_SYNC_MANIFEST='$workspace/.kd414-sync-manifest' bash '$workspace/scripts/remote/validate-kd414-native.sh' --workspace '$workspace' --expected-arch '$arch' --expected-fingerprint '$fp'" \
    >"$log" 2>&1; then
    echo "PASS: $label"
  else
    echo "FAIL: $label" >&2
    tail -60 "$log" >&2 || true
    exit 1
  fi
}

echo "KD4.14 native gate run_id=$RUN_ID"
preflight_ssh
compute_fingerprint
write_sync_manifest
EXPECTED_FP="$(extract_fingerprint)"
[[ -n "$EXPECTED_FP" ]] || exit 2
echo "expected_fingerprint=$EXPECTED_FP"

rsync_tree "$NETCUP_HOST" "$NETCUP_WORKSPACE"
copy_scripts "$NETCUP_HOST" "$NETCUP_WORKSPACE"
run_remote "netcup-amd64" "$NETCUP_HOST" "$NETCUP_WORKSPACE" x86_64 \
  "$RESULTS_DIR/netcup-amd64.log" "$EXPECTED_FP"

if [[ "$RUN_ORACLE" == "1" ]]; then
  rsync_tree "$ORACLE_HOST" "$ORACLE_WORKSPACE"
  copy_scripts "$ORACLE_HOST" "$ORACLE_WORKSPACE"
  run_remote "oracle-aarch64" "$ORACLE_HOST" "$ORACLE_WORKSPACE" aarch64 \
    "$RESULTS_DIR/oracle-aarch64.log" "$EXPECTED_FP"
fi

echo "KD4.14 native gate complete: $RESULTS_DIR"
echo "fingerprint=$EXPECTED_FP"
echo "SAME_FINGERPRINT=YES"
