#!/usr/bin/env bash
# P1.5-WS6 — Dual-arch release orchestrator (Mac → Netcup amd64 + Oracle arm64).
# Linux evidence only on remote hosts. BIT_FOR_BIT NOT_CLAIMED.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30)
ONLY_ARCH="${P15_WS6_ONLY_ARCH:-}"
LOCK_ID="${P15_WS6_LOCK_ID:-}"
ARTIFACT_DIR=""

usage() {
  cat <<'EOF'
Usage: p15-ws6-dual-arch.sh [--artifact-dir PATH] [--arch amd64|arm64|all] [--lock-id ID]
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
[[ -n "$LOCK_ID" ]] || LOCK_ID="P1_5_WS6_LOCK_${TS}_${HEAD12}"

EV_ROOT="$ROOT/docs/operations/evidence/p1.5-ws6"
[[ -n "$ARTIFACT_DIR" ]] || ARTIFACT_DIR="$EV_ROOT/$LOCK_ID"
[[ "$ARTIFACT_DIR" = /* ]] || ARTIFACT_DIR="$ROOT/$ARTIFACT_DIR"
mkdir -p "$ARTIFACT_DIR/orchestrator" "$EV_ROOT"
echo "$LOCK_ID" >"$EV_ROOT/LATEST_LOCK_ID"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-p15-ws6-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-p15-ws6-src}"

hash_files() {
  # shellcheck disable=SC2086
  shasum -a 256 "$@" 2>/dev/null | shasum -a 256 | awk '{print $1}'
}

HARNESS_HASH="$(hash_files \
  scripts/release/p15-ws6-dual-arch.sh \
  scripts/release/p15-ws6-release-artifact-validation.sh \
  scripts/release/p15-ws6-build-artifacts.sh \
  scripts/release/p15-ws6-reproducibility-check.sh \
  scripts/release/p15-ws6-verify-checksums.sh \
  scripts/release/p15-ws6-generate-sbom.sh \
  scripts/release/p15-ws6-verify-sbom.sh \
  scripts/release/p15-ws6-install-verify.sh \
  scripts/release/lib/ws6-common.sh 2>/dev/null || echo none)"

PACKAGING_CONTRACT_HASH="$(hash_files \
  docs/release/p1.5-packaging-product-contract.md \
  docs/release/build-manifest.md \
  docs/release/reproducible-builds.md 2>/dev/null || echo none)"

WORKING_TREE_STATUS=DIRTY
git diff --quiet && git diff --cached --quiet && WORKING_TREE_STATUS=CLEAN || true

cat >"$EV_ROOT/lock.env" <<EOF
P1_5_WS6_LOCK_ID=$LOCK_ID
HEAD=$HEAD
WORKING_TREE_STATUS=$WORKING_TREE_STATUS
HARNESS_HASH=$HARNESS_HASH
PACKAGING_CONTRACT_HASH=$PACKAGING_CONTRACT_HASH
BIT_FOR_BIT_CLAIM=NOT_CLAIMED
HOST=orchestrator_mac
ARCH=$(uname -m)
STARTED_AT=$TS
NETCUP_WS=$NETCUP_WS
ORACLE_WS=$ORACLE_WS
HARNESS_HASH_RECIPE=shasum -a 256 scripts/release/p15-ws6-*.sh scripts/release/lib/ws6-common.sh | shasum -a 256
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
    --exclude 'docs/operations/evidence/p1.5-ws6/*/amd64' \
    --exclude 'docs/operations/evidence/p1.5-ws6/*/arm64' \
    "$ROOT/" "${host}:${workspace}/"
  # Inject SOURCE_HEAD because .git is excluded from rsync.
  ssh "${SSH_OPTS[@]}" "$host" "printf '%s\n' '$HEAD' >'$workspace/.ws6-source-head'; \
    printf '%s\n' '$WORKING_TREE_STATUS' >'$workspace/.ws6-source-tree-status'"
}

run_remote_ws6() {
  local host="$1" label="$2" arch="$3" workspace="$4"
  local local_dir="$ARTIFACT_DIR/$label"
  local remote_art="$workspace/docs/operations/evidence/p1.5-ws6/$LOCK_ID/$label"
  mkdir -p "$local_dir"
  local log="$ARTIFACT_DIR/orchestrator/${label}-ws6.log"
  local rc=0

  echo "=== WS6 $label on $host ===" | tee "$log"

  if ! ssh "${SSH_OPTS[@]}" "$host" \
    "source ~/.cargo/env 2>/dev/null || true; cd '$workspace'; \
     export EXYONQ_SOURCE_REVISION='$HEAD'; \
     export EXYONQ_SOURCE_TREE_STATUS='$WORKING_TREE_STATUS'; \
     mkdir -p '$remote_art'; \
     bash scripts/release/p15-ws6-release-artifact-validation.sh \
       --workspace '$workspace' --host-label '$label' --expected-arch '$arch' \
       --artifact-dir '$remote_art'" \
    >>"$log" 2>&1; then
    rc=1
  fi

  rsync -az -e "ssh ${SSH_OPTS[*]}" \
    "${host}:${remote_art}/" \
    "$local_dir/" 2>/dev/null || true

  echo "REMOTE_RC_${label}_WS6=$rc" | tee -a "$log"
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
  if ! run_remote_ws6 "$host" "$label" "$arch" "$workspace"; then
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
    "DUAL_ARCH_WS6_RECONCILIATION": verdict,
    "bit_for_bit_claim": "NOT_CLAIMED",
    "notes": notes,
}
(root / "summary.json").write_text(json.dumps(out, indent=2) + "\n")
(root / "summary.txt").write_text(
    f"LOCK={lock}\nDUAL_ARCH_WS6_RECONCILIATION={verdict}\nBIT_FOR_BIT=NOT_CLAIMED\n"
    f"AMD64={a and a.get('verdict')}\nARM64={b and b.get('verdict')}\n"
)
print(verdict)
PY

cat "$ARTIFACT_DIR/summary.txt"
echo "P15_WS6_LOCK=$LOCK_ID ARTIFACT_DIR=$ARTIFACT_DIR RC=$RC"
exit "$RC"
