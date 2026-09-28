#!/usr/bin/env bash
# Cap062 dual-Linux orchestrator: Netcup amd64 + Oracle arm64 in parallel.
# Builds packages on each authority and runs tarball+deb real E2E there.
# RPM install remains blocked until RPM-family hosts exist.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-phase1-cap062-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-phase1-cap062-src}"
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30)

HEAD="$(git rev-parse HEAD)"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
RUN_ID="${CAP062_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap062-${RUN_ID}"
mkdir -p "$EVIDENCE"/{amd64,arm64,orchestrator} "$ROOT/.exyonq-local/status" "$ROOT/.exyonq-local/logs"

cat >"$EVIDENCE/orchestrator/run_meta.txt" <<EOF
CAPABILITY_ID=062
FEATURE_ID=pkg-linux-amd64-arm64
CAPABILITY_062_STARTED=YES
OPTION=C
HEAD=$HEAD
TREE=$TREE
CARGO_LOCK_SHA256=$LOCK_SHA
RUN_ID=$RUN_ID
CAP062_OCI_INCLUDED=NO
CAP062_RPM_REAL_ENVIRONMENT_BLOCKER=OWNER_INFRASTRUCTURE_REQUIRED
CAP061=CLOSED_VERIFIED_REAL_PRODUCTION
LA_CAP054_008=OPEN
CAP067_STARTED=NO
PROJECT_PHASE_1_STATUS=NOT_CLOSED
R6_STATUS=NOT_STARTED
PUSH=NO
GHCR_WRITE=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
EOF

echo "RUN_ID=$RUN_ID HEAD=$HEAD"
echo "CAP062_RPM_REAL_ENVIRONMENT_BLOCKER=OWNER_INFRASTRUCTURE_REQUIRED"

sync_tree() {
  local host="$1" workspace="$2"
  echo "[$(date -u +%H:%M:%S)] rsync → ${host}:${workspace}"
  ssh "${SSH_OPTS[@]}" "$host" "mkdir -p '$workspace'"
  # Keep .git for HEAD identity on host (needed by ws6_git_head / EXYONQ_SOURCE_REVISION).
  rsync -az --delete \
    -e "ssh ${SSH_OPTS[*]}" \
    --exclude 'target/' \
    --exclude 'benchmarks/results/' \
    --exclude 'benchmarks/results-dev/' \
    --exclude '.exyonq-local/tmp/' \
    --exclude '.exyonq-local/logs/' \
    "$ROOT/" "${host}:${workspace}/"
  # Stamp source head for hosts where .git may be incomplete after partial sync.
  ssh "${SSH_OPTS[@]}" "$host" "printf '%s\n' '$HEAD' >'$workspace/.ws6-source-head'"
}

run_host() {
  local label="$1" host="$2" workspace="$3" arch="$4"
  local log="$EVIDENCE/orchestrator/${label}.log"
  echo "[$(date -u +%H:%M:%S)] start $label on $host"
  set +e
  ssh "${SSH_OPTS[@]}" "$host" \
    "CAP062_RUN_ID='$RUN_ID' CAP062_PURGE='${CAP062_PURGE:-1}' \
     CAP062_OUT_DIR='$workspace/.exyonq-local/tmp/phase1-cap062-${RUN_ID}/${arch}' \
     bash '$workspace/scripts/reality/cap062-host-build-and-e2e.sh'" \
    >"$log" 2>&1
  local rc=$?
  set -e
  mkdir -p "$EVIDENCE/$arch"
  # Fetch summary + artifacts listing (not full multi-hundred-MB binaries unless needed)
  scp -q "${SSH_OPTS[@]}" \
    "${host}:${workspace}/.exyonq-local/tmp/phase1-cap062-${RUN_ID}/${arch}/summary.json" \
    "$EVIDENCE/$arch/summary.json" 2>/dev/null || \
    echo '{"OVERALL":"FAIL","error":"summary missing"}' >"$EVIDENCE/$arch/summary.json"
  scp -q "${SSH_OPTS[@]}" \
    "${host}:${workspace}/.exyonq-local/tmp/phase1-cap062-${RUN_ID}/${arch}/steps.jsonl" \
    "$EVIDENCE/$arch/steps.jsonl" 2>/dev/null || true
  scp -q "${SSH_OPTS[@]}" \
    "${host}:${workspace}/.exyonq-local/tmp/phase1-cap062-${RUN_ID}/${arch}/artifacts/SHA256SUMS.txt" \
    "$EVIDENCE/$arch/SHA256SUMS.txt" 2>/dev/null || true
  # Fetch package files (deb/rpm/tarball) for cross-arch wrong-arch tests / archive
  mkdir -p "$EVIDENCE/$arch/artifacts"
  # Fetch packages via remote tar to avoid scp glob pitfalls.
  ssh "${SSH_OPTS[@]}" "$host" \
    "cd '$workspace/.exyonq-local/tmp/phase1-cap062-${RUN_ID}/${arch}/artifacts' && tar -cf - \
      \$(ls *.deb *.rpm exyonq-*-linux-${arch}.tar.gz exyonq-linux-${arch}.tar.gz SHA256SUMS.txt 2>/dev/null || true)" \
    2>/dev/null | tar -xf - -C "$EVIDENCE/$arch/artifacts" 2>/dev/null || true
  echo "$rc" >"$EVIDENCE/orchestrator/${label}.exit"
  echo "[$(date -u +%H:%M:%S)] done $label rc=$rc"
  return "$rc"
}

sync_tree "$NETCUP_HOST" "$NETCUP_WS"
sync_tree "$ORACLE_HOST" "$ORACLE_WS"

# Parallel host runs
set +e
run_host amd64 "$NETCUP_HOST" "$NETCUP_WS" amd64 &
PID_A=$!
run_host arm64 "$ORACLE_HOST" "$ORACLE_WS" arm64 &
PID_B=$!
wait "$PID_A"; RC_A=$?
wait "$PID_B"; RC_B=$?
set -e

# Cross-arch wrong-arch check: present arm64 FHS tarball to amd64 host
WRONG=""
for cand in "$EVIDENCE"/arm64/artifacts/exyonq-*-linux-arm64.tar.gz; do
  if [[ -f "$cand" ]]; then
    WRONG="$cand"
    break
  fi
done
if [[ -n "$WRONG" ]]; then
  scp -q "${SSH_OPTS[@]}" "$WRONG" "${NETCUP_HOST}:/tmp/cap062-wrong-arch.tgz"
  set +e
  ssh "${SSH_OPTS[@]}" "$NETCUP_HOST" \
    "bash '$NETCUP_WS/scripts/release/p15-ws6-install-verify.sh' \
      --artifact /tmp/cap062-wrong-arch.tgz \
      --staging-root /tmp/cap062-wrong-arch-staging" \
    >"$EVIDENCE/orchestrator/wrong-arch-amd64-host.log" 2>&1
  WRC=$?
  set -e
  if [[ "$WRC" -eq 0 ]]; then
    echo "WRONG_ARCH_REJECT=FAIL" | tee "$EVIDENCE/orchestrator/wrong-arch.txt"
    RC_A=1
  else
    echo "WRONG_ARCH_REJECT=PASS" | tee "$EVIDENCE/orchestrator/wrong-arch.txt"
  fi
else
  echo "WRONG_ARCH_REJECT=FAIL_MISSING_ARM64_ARTIFACT" | tee "$EVIDENCE/orchestrator/wrong-arch.txt"
  RC_B=1
fi

python3 - "$EVIDENCE/orchestrator/aggregate.json" <<PY
import json, pathlib
ev = pathlib.Path("$EVIDENCE")
def load(p):
    try:
        return json.loads(pathlib.Path(p).read_text())
    except Exception as e:
        return {"OVERALL":"FAIL","error":str(e)}
def step_status(summary, name):
    for s in summary.get("steps") or []:
        if s.get("step") == name:
            return s.get("status")
    return "MISSING"

def surface_pass(summary, names):
    statuses = [step_status(summary, n) for n in names]
    if any(st == "FAIL" for st in statuses):
        return "FAIL"
    if any(st == "MISSING" for st in statuses):
        return "FAIL"
    if all(st in ("PASS", "WARN", "SKIP") for st in statuses) and any(st == "PASS" for st in statuses):
        return "PASS"
    return summary.get("OVERALL", "FAIL")

amd = load(ev/"amd64"/"summary.json")
arm = load(ev/"arm64"/"summary.json")
wrong = "UNKNOWN"
wp = ev/"orchestrator"/"wrong-arch.txt"
if wp.exists():
    wrong = wp.read_text().strip()
obj = {
  "CAPABILITY_ID": "062",
  "RUN_ID": "$RUN_ID",
  "HEAD": "$HEAD",
  "TREE": "$TREE",
  "CARGO_LOCK_SHA256": "$LOCK_SHA",
  "NETCUP_AMD64_EXIT": $RC_A,
  "ORACLE_ARM64_EXIT": $RC_B,
  "amd64": amd,
  "arm64": arm,
  "WRONG_ARCH_REJECT": wrong,
  "CAP062_RPM_REAL_ENVIRONMENT_BLOCKER": "OWNER_INFRASTRUCTURE_REQUIRED",
  "TARBALL_AMD64": surface_pass(amd, ["FHS_RUNTIME", "FHS_BINARY_IDENTITY", "FHS_ELF"]),
  "TARBALL_ARM64": surface_pass(arm, ["FHS_RUNTIME", "FHS_BINARY_IDENTITY", "FHS_ELF"]),
  "DEB_AMD64": surface_pass(amd, ["DEB_INSTALL", "DEB_SYSTEMD_ACTIVE", "DEB_HTTP", "DEB_BINARY_IDENTITY"]),
  "DEB_ARM64": surface_pass(arm, ["DEB_INSTALL", "DEB_SYSTEMD_ACTIVE", "DEB_HTTP", "DEB_BINARY_IDENTITY"]),
  "RPM_AMD64": "BLOCKED_NO_RPM_HOST",
  "RPM_ARM64": "BLOCKED_NO_RPM_HOST",
  "CAP062_OCI_INCLUDED": "NO",
  "CAPABILITY_062_TERMINAL_READY": False,
  "NOTE": "RPM install/runtime blocked; Cap062 cannot close VERIFIED_REAL_PRODUCTION until RPM-family dual-arch authorities exist. Generation of .rpm artifacts is still required and should appear in host summaries.",
}
pathlib.Path("$EVIDENCE/orchestrator/aggregate.json").write_text(json.dumps(obj, indent=2)+"\n")
print(json.dumps(obj, indent=2))
PY

cp "$EVIDENCE/orchestrator/aggregate.json" "$ROOT/.exyonq-local/status/CAP062-LATEST-AGGREGATE.json"
echo "EVIDENCE=$EVIDENCE"

if [[ "$RC_A" -ne 0 || "$RC_B" -ne 0 ]]; then
  echo "CAP062_DUAL_LINUX=FAIL (host e2e)"
  exit 1
fi
echo "CAP062_DUAL_LINUX=PASS_TARBALL_DEB (RPM still BLOCKED)"
exit 0
