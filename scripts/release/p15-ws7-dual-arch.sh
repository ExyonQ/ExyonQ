#!/usr/bin/env bash
# P1.5-WS7 — Dual-arch final reconciliation orchestrator (Mac → Netcup + Oracle).
# Scoped F01–F16 only. BIT_FOR_BIT NOT_CLAIMED. No publish. No commit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30)
ONLY_ARCH="${P15_WS7_ONLY_ARCH:-}"
LOCK_ID="${P15_WS7_LOCK_ID:-}"
ARTIFACT_DIR=""

usage() {
  cat <<'EOF'
Usage: p15-ws7-dual-arch.sh [--artifact-dir PATH] [--arch amd64|arm64|all] [--lock-id ID]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --arch) ONLY_ARCH="$2"; shift 2 ;;
    --lock-id) LOCK_ID="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

HEAD="$(git rev-parse HEAD)"
HEAD12="$(git rev-parse --short=12 HEAD)"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
[[ -n "$LOCK_ID" ]] || LOCK_ID="P1_5_WS7_LOCK_${TS}_${HEAD12}"

EV_ROOT="$ROOT/docs/operations/evidence/p1.5-ws7"
[[ -n "$ARTIFACT_DIR" ]] || ARTIFACT_DIR="$EV_ROOT/$LOCK_ID"
[[ "$ARTIFACT_DIR" = /* ]] || ARTIFACT_DIR="$ROOT/$ARTIFACT_DIR"
mkdir -p "$ARTIFACT_DIR/orchestrator" "$EV_ROOT"
echo "$LOCK_ID" >"$EV_ROOT/LATEST_LOCK_ID"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-p15-ws7-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-p15-ws7-src}"

hash_files() {
  shasum -a 256 "$@" 2>/dev/null | shasum -a 256 | awk '{print $1}'
}

HARNESS_HASH="$(hash_files \
  scripts/release/p15-ws7-dual-arch.sh \
  scripts/release/p15-ws7-final-reconciliation.sh \
  scripts/release/lib/ws6-common.sh 2>/dev/null || echo none)"

DOCS_HASH="$(hash_files \
  docs/architecture/p1.5-final-capability-matrix.md \
  docs/architecture/p1.5-final-claims-and-limits.md \
  docs/architecture/p1.5-final-close-report.md \
  docs/security/p1.5-final-findings-reconciliation.md \
  docs/operations/p1.5-final-commit-unit-manifest.md \
  docs/operations/p1.5-final-evidence-index.md \
  docs/operations/p1.5-ws7-dual-arch-reconciliation.md 2>/dev/null || echo none)"

SCOPE_MANIFEST_HASH="$(hash_files docs/operations/p1.5-final-commit-unit-manifest.md 2>/dev/null || echo none)"
DEPENDENCY_GATE_HASH="$(hash_files \
  docs/security/dependency-admissions-drift.md \
  scripts/architecture/verify-dependency-containment.sh 2>/dev/null || echo none)"

WORKING_TREE_STATUS=DIRTY
git diff --quiet && git diff --cached --quiet && WORKING_TREE_STATUS=CLEAN || true

cat >"$EV_ROOT/lock.env" <<EOF
P1_5_WS7_LOCK_ID=$LOCK_ID
HEAD=$HEAD
WORKING_TREE_STATUS=$WORKING_TREE_STATUS
WS1_LOCK=P1_5_WS1_LOCK_20260720T152416Z_04656c9cd4f2
WS2_LOCK=P1_5_WS2_LOCK_20260720T164500Z_04656c9cd4f2
WS3_LOCK=P1_5_WS3_LOCK_20260720T171247Z_04656c9cd4f2
WS4_LOCK=P1_5_WS4_LOCK_20260720T183445Z_04656c9cd4f2
WS5_LOCK=P1_5_WS5_LOCK_20260720T224635Z_04656c9cd4f2
WS6_LOCK=P1_5_WS6_LOCK_20260721T001809Z_04656c9cd4f2
FINAL_HARNESS_HASH=$HARNESS_HASH
DOCS_HASH=$DOCS_HASH
SCOPE_MANIFEST_HASH=$SCOPE_MANIFEST_HASH
DEPENDENCY_GATE_HASH=$DEPENDENCY_GATE_HASH
BIT_FOR_BIT_CLAIM=NOT_CLAIMED
HOST=orchestrator_mac
ARCH=$(uname -m)
STARTED_AT=$TS
NETCUP_WS=$NETCUP_WS
ORACLE_WS=$ORACLE_WS
LINUX_EVIDENCE=Netcup_amd64_and_Oracle_arm64
IMAC_DOCKER=LOCAL_ITERATION_ONLY_NOT_LINUX_EVIDENCE
EOF
cp "$EV_ROOT/lock.env" "$ARTIFACT_DIR/lock.env"

sync_tree() {
  local host="$1" workspace="$2"
  echo "=== rsync → $host:$workspace ==="
  ssh "${SSH_OPTS[@]}" "$host" "mkdir -p '$workspace'"
  rsync -az --delete -e "ssh ${SSH_OPTS[*]}" \
    --exclude '.git' \
    --exclude 'target' \
    --exclude 'target/' \
    --exclude 'benchmarks/results' \
    --exclude 'benchmarks/results-dev' \
    --exclude 'docs/operations/evidence/p1.5-ws6/*/amd64/build' \
    --exclude 'docs/operations/evidence/p1.5-ws6/*/amd64/repro' \
    --exclude 'docs/operations/evidence/p1.5-ws6/*/amd64/sbom' \
    --exclude 'docs/operations/evidence/p1.5-ws6/*/arm64/build' \
    --exclude 'docs/operations/evidence/p1.5-ws6/*/arm64/repro' \
    --exclude 'docs/operations/evidence/p1.5-ws6/*/arm64/sbom' \
    --exclude 'docs/operations/evidence/p1.5-ws7/*/amd64' \
    --exclude 'docs/operations/evidence/p1.5-ws7/*/arm64' \
    "$ROOT/" "${host}:${workspace}/"
  ssh "${SSH_OPTS[@]}" "$host" "printf '%s\n' '$HEAD' >'$workspace/.ws6-source-head'; \
    printf '%s\n' '$WORKING_TREE_STATUS' >'$workspace/.ws6-source-tree-status'"
}

run_remote_ws7() {
  local host="$1" label="$2" arch="$3" workspace="$4"
  local local_dir="$ARTIFACT_DIR/$label"
  local remote_art="$workspace/docs/operations/evidence/p1.5-ws7/$LOCK_ID/$label"
  mkdir -p "$local_dir"
  local log="$ARTIFACT_DIR/orchestrator/${label}-ws7.log"
  local rc=0

  echo "=== WS7 $label on $host ===" | tee "$log"

  if ! ssh "${SSH_OPTS[@]}" "$host" \
    "source ~/.cargo/env 2>/dev/null || true; cd '$workspace'; \
     export EXYONQ_SOURCE_REVISION='$HEAD'; \
     export EXYONQ_SOURCE_TREE_STATUS='$WORKING_TREE_STATUS'; \
     mkdir -p '$remote_art'; \
     chmod +x scripts/release/p15-ws7-final-reconciliation.sh; \
     bash scripts/release/p15-ws7-final-reconciliation.sh \
       --workspace '$workspace' --host-label '$label' --expected-arch '$arch' \
       --artifact-dir '$remote_art'" \
    >>"$log" 2>&1; then
    rc=1
  fi

  rsync -az -e "ssh ${SSH_OPTS[*]}" \
    "${host}:${remote_art}/" \
    "$local_dir/" 2>/dev/null || true

  echo "REMOTE_RC_${label}_WS7=$rc" | tee -a "$log"
  echo "$rc" >"$local_dir/rc.txt"

  python3 - "$local_dir" "$label" "$LOCK_ID" "$rc" <<'PY'
import json, pathlib, sys
arch_dir = pathlib.Path(sys.argv[1])
label, lock, rc = sys.argv[2:5]
rc = int(rc)
summary = arch_dir / "summary.json"
data = json.loads(summary.read_text()) if summary.exists() else None
verdict = "PASS"
notes = []
if rc != 0:
    verdict = "FAIL"
    notes.append("remote_rc_nonzero")
if data and data.get("verdict") != "PASS":
    verdict = "FAIL"
    notes.append("validation_fail")
out = {
    "lock_id": lock,
    "arch_label": label,
    "remote_rc": rc,
    "validation": data,
    "notes": notes,
    "bit_for_bit_claim": "NOT_CLAIMED",
    "verdict": verdict,
}
(arch_dir / "arch-summary.json").write_text(json.dumps(out, indent=2) + "\n")
(arch_dir / "arch-summary.txt").write_text(
    f"LOCK={lock}\nARCH={label}\nVERDICT={verdict}\nREMOTE_RC={rc}\nBIT_FOR_BIT=NOT_CLAIMED\n"
)
print(verdict)
PY
  return "$rc"
}

RC=0
ARCHES=()
[[ -z "$ONLY_ARCH" || "$ONLY_ARCH" == "all" || "$ONLY_ARCH" == "amd64" ]] && \
  ARCHES+=("amd64|$NETCUP_HOST|x86_64|$NETCUP_WS")
[[ -z "$ONLY_ARCH" || "$ONLY_ARCH" == "all" || "$ONLY_ARCH" == "arm64" ]] && \
  ARCHES+=("arm64|$ORACLE_HOST|aarch64|$ORACLE_WS")

for spec in "${ARCHES[@]}"; do
  IFS='|' read -r label host arch workspace <<<"$spec"
  sync_tree "$host" "$workspace" | tee "$ARTIFACT_DIR/orchestrator/${label}-rsync.log"
  if ! run_remote_ws7 "$host" "$label" "$arch" "$workspace"; then
    RC=1
  fi
done

python3 - "$ARTIFACT_DIR" "$LOCK_ID" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
lock = sys.argv[2]
amd = root / "amd64" / "arch-summary.json"
arm = root / "arm64" / "arch-summary.json"
def load(p):
    return json.loads(p.read_text()) if p.exists() else None
a, b = load(amd), load(arm)
verdict = "INCOMPLETE"
notes = []
if a and b:
    verdict = "PASS" if a.get("verdict") == "PASS" and b.get("verdict") == "PASS" else "FAIL"
    if verdict == "FAIL":
        notes.append("one or both arches FAIL")
elif a or b:
    verdict = "PARTIAL"
    notes.append("missing one arch")
out = {
    "lock_id": lock,
    "amd64": a,
    "arm64": b,
    "DUAL_ARCH_WS7_FINAL_RECONCILIATION": verdict,
    "bit_for_bit_claim": "NOT_CLAIMED",
    "notes": notes,
}
(root / "summary.json").write_text(json.dumps(out, indent=2) + "\n")
(root / "summary.txt").write_text(
    f"LOCK={lock}\nDUAL_ARCH_WS7_FINAL_RECONCILIATION={verdict}\nBIT_FOR_BIT=NOT_CLAIMED\n"
    f"AMD64={a and a.get('verdict')}\nARM64={b and b.get('verdict')}\n"
)
print(verdict)
PY

cat "$ARTIFACT_DIR/summary.txt"
echo "P15_WS7_LOCK=$LOCK_ID ARTIFACT_DIR=$ARTIFACT_DIR RC=$RC"
exit "$RC"
