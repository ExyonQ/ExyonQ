#!/usr/bin/env bash
# P1.1 Mac orchestrator: sync local tree, run bounded soak on amd64 + arm64, pull reports.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${P1_1_SOAK_SSH_OPTS:-${P11_SOAK_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}}"
RUN_ID="${P1_1_SOAK_RUN_ID:-${P11_SOAK_RUN_ID:-p1-1-soak-$(date -u +%Y%m%dT%H%M%SZ)}}"
DURATION_SEC="${P1_1_SOAK_DURATION_SEC:-${P11_SOAK_DURATION_SEC:-900}}"
REMOTE_SCRIPT="scripts/soak/p1.1-correctness-stability-soak-remote.sh"
ORCH_DIR="$ROOT/docs/operations/p1.1-soak-evidence/$RUN_ID/orchestrator"
LOCAL_COMMIT="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
LOCAL_BRANCH="$(git branch --show-current 2>/dev/null || echo unknown)"
mkdir -p "$ORCH_DIR"

HOST_SPECS=(
  "${NETCUP_HOST:-netcup-bench}|amd64|x86_64|${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}|docs/operations/p1.1-soak-amd64-report.md"
  "${ORACLE_HOST:-oracle-quasar}|arm64|aarch64|${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-dev-soak-src}|docs/operations/p1.1-soak-arm64-report.md"
)

sync_tree() {
  local host="$1"
  local workspace="$2"
  echo "=== rsync to $host:$workspace ==="
  rsync -az --delete -e "ssh $SSH_OPTS" \
    --exclude '.git' \
    --exclude 'target' \
    --exclude 'target/' \
    --exclude 'benchmarks/results' \
    --exclude 'benchmarks/results-dev' \
    --exclude 'docs/operations/p1.1-soak-evidence' \
    "$ROOT/" "${host}:${workspace}/"
}

run_host() {
  local spec="$1"
  local host label arch workspace report_rel
  IFS='|' read -r host label arch workspace report_rel <<<"$spec"
  local log="$ORCH_DIR/${label}.log"

  sync_tree "$host" "$workspace" | tee "$ORCH_DIR/${label}-rsync.log"

  echo "=== remote soak $label ($host) ===" | tee "$log"
  local rc=0
  if ssh $SSH_OPTS "$host" \
    "source ~/.cargo/env 2>/dev/null || true; chmod +x '$workspace/$REMOTE_SCRIPT'; P1_1_SOAK_RUN_ID='$RUN_ID' P1_1_SOAK_COMMIT='$LOCAL_COMMIT' P1_1_SOAK_BRANCH='$LOCAL_BRANCH' bash '$workspace/$REMOTE_SCRIPT' --workspace '$workspace' --host-label '$label' --expected-arch '$arch' --duration-sec '$DURATION_SEC' --report-relpath '$report_rel'" \
    >>"$log" 2>&1; then
    echo "REMOTE_RC_${label}=0" | tee -a "$log"
  else
    rc=$?
    echo "REMOTE_RC_${label}=$rc" | tee -a "$log"
  fi

  mkdir -p "$ROOT/docs/operations/p1.1-soak-evidence/$RUN_ID/$label"
  rsync -az -e "ssh $SSH_OPTS" \
    "${host}:${workspace}/docs/operations/p1.1-soak-evidence/${RUN_ID}/${label}/" \
    "$ROOT/docs/operations/p1.1-soak-evidence/$RUN_ID/$label/" 2>/dev/null || true
  rsync -az -e "ssh $SSH_OPTS" \
    "${host}:${workspace}/${report_rel}" \
    "$ROOT/${report_rel}" 2>/dev/null || true

  if [[ -f "$ROOT/${report_rel}" ]]; then
    grep -m1 '^\*\*' "$ROOT/${report_rel}" | sed "s/^/${label}: /" | tee -a "$log"
  fi
  return "$rc"
}

echo "P1.1 soak run_id=$RUN_ID duration_sec=$DURATION_SEC"
echo "Reports:"
echo "- docs/operations/p1.1-soak-amd64-report.md"
echo "- docs/operations/p1.1-soak-arm64-report.md"

RC=0
if [[ "${P1_1_SOAK_PARALLEL:-${P11_SOAK_PARALLEL:-0}}" == "1" ]]; then
  pids=()
  for spec in "${HOST_SPECS[@]}"; do
    run_host "$spec" &
    pids+=("$!")
  done
  for pid in "${pids[@]}"; do
    wait "$pid" || RC=1
  done
else
  for spec in "${HOST_SPECS[@]}"; do
    run_host "$spec" || RC=1
  done
fi

echo "P1.1 soak complete run_id=$RUN_ID rc=$RC"
exit "$RC"
