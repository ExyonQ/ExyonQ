#!/usr/bin/env bash
# P1.2 Mac orchestrator: sync frozen dirty tree, soak amd64 then arm64 (sequential), pull evidence.
# Not a competitive benchmark. No HTML. COMMIT/PUSH not performed here.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS="${P1_2_SOAK_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30}"
LOCK_ID="${P1_2_SOAK_LOCK_ID:-$(cat "$ROOT/docs/operations/evidence/p1.2-soak/LATEST_LOCK_ID" 2>/dev/null || echo "p12-soak-$(date -u +%Y%m%dT%H%M%SZ)")}"
RUN_ID="${P1_2_SOAK_RUN_ID:-$LOCK_ID}"
DURATION_SEC="${P1_2_SOAK_DURATION_SEC:-1200}"
WARMUP_SEC="${P1_2_SOAK_WARMUP_SEC:-60}"
REMOTE_SCRIPT="scripts/soak/p1.2-fastcgi-wordpress-soak-remote.sh"
ORCH_DIR="$ROOT/docs/operations/evidence/p1.2-soak/$RUN_ID/orchestrator"
LOCAL_COMMIT="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
LOCAL_BRANCH="$(git branch --show-current 2>/dev/null || echo unknown)"
mkdir -p "$ORCH_DIR"

# Sequential by default — one host at a time (rule 104 / no parallel soaks on shared methodology).
HOST_SPECS=(
  "${NETCUP_HOST:-netcup-bench}|amd64|x86_64|${NETCUP_WORKSPACE:-/root/exyonq-p12-soak-src}|docs/operations/p1.2-fastcgi-soak-amd64-report.md|16"
  "${ORACLE_HOST:-oracle-quasar}|arm64|aarch64|${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-p12-soak-src}|docs/operations/p1.2-fastcgi-soak-arm64-report.md|8"
)

ONLY_HOST="${P1_2_SOAK_ONLY:-}"

sync_tree() {
  local host="$1"
  local workspace="$2"
  echo "=== rsync to $host:$workspace ==="
  ssh $SSH_OPTS "$host" "mkdir -p '$workspace'"
  rsync -az --delete -e "ssh $SSH_OPTS" \
    --exclude '.git' \
    --exclude 'target' \
    --exclude 'target/' \
    --exclude 'benchmarks/results' \
    --exclude 'benchmarks/results-dev' \
    --exclude 'docs/operations/p1.1-soak-evidence' \
    --exclude 'docs/operations/evidence/p1.2-soak/*/amd64' \
    --exclude 'docs/operations/evidence/p1.2-soak/*/arm64' \
    --exclude 'docs/operations/evidence/p1.2-wordpress-no-cache' \
    "$ROOT/" "${host}:${workspace}/"
}

run_host() {
  local spec="$1"
  local host label arch workspace report_rel conc
  IFS='|' read -r host label arch workspace report_rel conc <<<"$spec"
  local log="$ORCH_DIR/${label}.log"

  if [[ -n "$ONLY_HOST" && "$label" != "$ONLY_HOST" ]]; then
    echo "SKIP host $label (P1_2_SOAK_ONLY=$ONLY_HOST)"
    return 0
  fi

  sync_tree "$host" "$workspace" | tee "$ORCH_DIR/${label}-rsync.log"

  echo "=== remote soak $label ($host) duration=${DURATION_SEC}s warmup=${WARMUP_SEC}s conc=$conc ===" | tee "$log"
  local rc=0
  if ssh $SSH_OPTS "$host" \
    "source ~/.cargo/env 2>/dev/null || true; chmod +x '$workspace/$REMOTE_SCRIPT'; \
     P1_2_SOAK_LOCK_ID='$LOCK_ID' P1_2_SOAK_RUN_ID='$RUN_ID' \
     P1_2_SOAK_COMMIT='$LOCAL_COMMIT' P1_2_SOAK_BRANCH='$LOCAL_BRANCH' \
     P1_2_SOAK_DURATION_SEC='$DURATION_SEC' P1_2_SOAK_WARMUP_SEC='$WARMUP_SEC' \
     P1_2_SOAK_CONCURRENCY='$conc' \
     bash '$workspace/$REMOTE_SCRIPT' \
       --workspace '$workspace' \
       --host-label '$label' \
       --expected-arch '$arch' \
       --duration-sec '$DURATION_SEC' \
       --warmup-sec '$WARMUP_SEC' \
       --concurrency '$conc' \
       --report-relpath '$report_rel'" \
    >>"$log" 2>&1; then
    echo "REMOTE_RC_${label}=0" | tee -a "$log"
  else
    rc=$?
    echo "REMOTE_RC_${label}=$rc" | tee -a "$log"
  fi

  mkdir -p "$ROOT/docs/operations/evidence/p1.2-soak/$RUN_ID/$label"
  rsync -az -e "ssh $SSH_OPTS" \
    "${host}:${workspace}/docs/operations/evidence/p1.2-soak/${RUN_ID}/${label}/" \
    "$ROOT/docs/operations/evidence/p1.2-soak/$RUN_ID/$label/" 2>/dev/null || true
  rsync -az -e "ssh $SSH_OPTS" \
    "${host}:${workspace}/${report_rel}" \
    "$ROOT/${report_rel}" 2>/dev/null || true

  if [[ -f "$ROOT/${report_rel}" ]]; then
    grep -E '^(VERDICT|AMD64_SOAK|ARM64_SOAK|\*\*)' "$ROOT/${report_rel}" | head -20 | sed "s/^/${label}: /" | tee -a "$log" || true
  fi
  return "$rc"
}

echo "P1.2 soak run_id=$RUN_ID lock=$LOCK_ID duration_sec=$DURATION_SEC warmup_sec=$WARMUP_SEC"
echo "PROGRAM_HEAD=$LOCAL_COMMIT branch=$LOCAL_BRANCH"
echo "Reports:"
echo "- docs/operations/p1.2-fastcgi-soak-amd64-report.md"
echo "- docs/operations/p1.2-fastcgi-soak-arm64-report.md"

RC=0
for spec in "${HOST_SPECS[@]}"; do
  run_host "$spec" || RC=1
done

echo "P1.2 soak complete run_id=$RUN_ID rc=$RC"
exit "$RC"
