#!/usr/bin/env bash
# P1.5-WS4 — Global Dual-Arch Soak orchestrator (Mac → Netcup amd64 + Oracle arm64).
# Not a competitive benchmark. No packaging. No WS5. COMMIT/PUSH not performed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30)
FORMAT=human
ARTIFACT_DIR=""
ONLY_SCENARIO=""
RUN_ALL=0
LIST_ONLY=0
DURATION_OVERRIDE=""
SAMPLE_INTERVAL="${P15_WS4_SAMPLE_INTERVAL:-5}"
ONLY_ARCH="${P15_WS4_ONLY_ARCH:-}"
LOCK_ID="${P15_WS4_LOCK_ID:-}"
PROFILE="${P15_WS4_PROFILE:-baseline}" # baseline | extended (extended not required for gate)

usage() {
  cat <<EOF
Usage: $0 [--list] [--scenario ID] [--all] [--duration SECONDS]
          [--sample-interval SECONDS] [--artifact-dir PATH]
          [--format human|json] [--arch amd64|arm64|all]
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list) LIST_ONLY=1; shift ;;
    --scenario) ONLY_SCENARIO="$2"; shift 2 ;;
    --all) RUN_ALL=1; shift ;;
    --duration) DURATION_OVERRIDE="$2"; shift 2 ;;
    --sample-interval) SAMPLE_INTERVAL="$2"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --format) FORMAT="$2"; shift 2 ;;
    --arch) ONLY_ARCH="$2"; shift 2 ;;
    --lock-id) LOCK_ID="$2"; shift 2 ;;
    --profile) PROFILE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

SCENARIOS=(
  "S01|Static|P11|static"
  "S02|Reverse proxy|P11|proxy"
  "S03|FastCGI|P12|fastcgi"
  "S04|WordPress no-cache|P12|wordpress"
  "S05|TLS HTTP/1.1|P13A|tls"
  "S06|HTTP/2|P13A|h2"
  "S07|HTTP/3|P13B|h3"
  "S08|Reload cycles|P11_P12_P13|reload"
  "S09|Drain cycles|P11_P12_P13|drain"
  "S10|Config tooling under service|THIN|tooling"
  "S11|FPM restart|P12|fpm"
  "S12|Mixed static/proxy/FastCGI|AGG|mixed_tcp"
  "S13|Mixed TLS/H2/H3|AGG|mixed_tls"
  "S14|Config tooling (alias S10)|THIN|tooling_alias"
  "S15|Resource-pressure bounded|THIN|pressure"
  "S16|Long idle + reconnect|THIN|idle"
)

if [[ "$LIST_ONLY" -eq 1 ]]; then
  printf '%s\n' "${SCENARIOS[@]}" | while IFS='|' read -r id title impl tag; do
    echo "$id  $title  ($impl)"
  done
  exit 0
fi

if [[ "$RUN_ALL" -eq 0 && -z "$ONLY_SCENARIO" ]]; then
  echo "Specify --all or --scenario <ID> (use --list)." >&2
  exit 2
fi

HEAD="$(git rev-parse HEAD)"
HEAD12="$(git rev-parse --short=12 HEAD)"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
[[ -n "$LOCK_ID" ]] || LOCK_ID="P1_5_WS4_LOCK_${TS}_${HEAD12}"

EV_ROOT="$ROOT/docs/operations/evidence/p1.5-ws4"
[[ -n "$ARTIFACT_DIR" ]] || ARTIFACT_DIR="$EV_ROOT/$LOCK_ID"
[[ "$ARTIFACT_DIR" = /* ]] || ARTIFACT_DIR="$ROOT/$ARTIFACT_DIR"
mkdir -p "$ARTIFACT_DIR/orchestrator" "$EV_ROOT"
echo "$LOCK_ID" >"$EV_ROOT/LATEST_LOCK_ID"

# Baseline durations (contract)
D_P11="${DURATION_OVERRIDE:-${P15_WS4_P11_SEC:-600}}"
D_P12="${DURATION_OVERRIDE:-${P15_WS4_P12_SEC:-600}}"
D_P13A="${DURATION_OVERRIDE:-${P15_WS4_P13A_SEC:-600}}"
D_P13B="${DURATION_OVERRIDE:-${P15_WS4_P13B_SEC:-400}}"
D_S10="${DURATION_OVERRIDE:-${P15_WS4_S10_DURATION_SEC:-120}}"
D_S15="${DURATION_OVERRIDE:-${P15_WS4_S15_DURATION_SEC:-180}}"
D_S16="${DURATION_OVERRIDE:-${P15_WS4_S16_DURATION_SEC:-120}}"
if [[ "$PROFILE" == "extended" ]]; then
  D_P11="${P15_WS4_EXT_P11_SEC:-14400}"
  D_P12="${P15_WS4_EXT_P12_SEC:-14400}"
  D_P13A="${P15_WS4_EXT_P13A_SEC:-14400}"
  D_P13B="${P15_WS4_EXT_P13B_SEC:-7200}"
fi

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WORKSPACE:-/root/exyonq-p15-ws4-src}"
ORACLE_WS="${ORACLE_WORKSPACE:-/home/ubuntu/exyonq-p15-ws4-src}"

hash_files() {
  # shellcheck disable=SC2086
  shasum -a 256 "$@" | shasum -a 256 | awk '{print $1}'
}

HARNESS_HASH="$(hash_files \
  scripts/soak/p15-ws4-global-soak.sh \
  scripts/soak/p15-ws4-thin-phases-remote.sh \
  scripts/soak/p1.1-correctness-stability-soak-remote.sh \
  scripts/soak/p1.2-fastcgi-wordpress-soak-remote.sh \
  scripts/soak/p1.3a-proxy-tls-h2-soak-remote.sh \
  scripts/soak/p1.3b-http3-proxy-soak-remote.sh)"
SCENARIO_MANIFEST_HASH="$(hash_files \
  docs/operations/p1.5-global-soak-contract.md \
  docs/operations/p1.5-global-soak-scenarios.md)"
CONFIG_HASH="$(hash_files \
  scripts/soak/lib/p13a-upstream.py 2>/dev/null || echo none)"
WORKING_TREE_STATUS=DIRTY
git diff --quiet && git diff --cached --quiet && WORKING_TREE_STATUS=CLEAN || true

cat >"$EV_ROOT/lock.env" <<EOF
P1_5_WS4_LOCK_ID=$LOCK_ID
HEAD=$HEAD
WORKING_TREE_STATUS=$WORKING_TREE_STATUS
HARNESS_HASH=$HARNESS_HASH
SCENARIO_MANIFEST_HASH=$SCENARIO_MANIFEST_HASH
CONFIG_HASH=$CONFIG_HASH
BINARY_HASH=built_on_host_release
FIXTURE_HASH=phase_native
LOADGEN_VERSION=phase_native_python_curl
HOST=orchestrator_mac
ARCH=$(uname -m)
STARTED_AT=$TS
DURATION_PROFILE=$PROFILE
D_P11=$D_P11
D_P12=$D_P12
D_P13A=$D_P13A
D_P13B=$D_P13B
D_S10=$D_S10
D_S15=$D_S15
D_S16=$D_S16
SAMPLE_INTERVAL=$SAMPLE_INTERVAL
NETCUP_WS=$NETCUP_WS
ORACLE_WS=$ORACLE_WS
HARNESS_HASH_RECIPE=shasum -a 256 scripts/soak/p15-ws4-global-soak.sh scripts/soak/p15-ws4-thin-phases-remote.sh scripts/soak/p1.1-correctness-stability-soak-remote.sh scripts/soak/p1.2-fastcgi-wordpress-soak-remote.sh scripts/soak/p1.3a-proxy-tls-h2-soak-remote.sh scripts/soak/p1.3b-http3-proxy-soak-remote.sh | shasum -a 256
SCENARIO_MANIFEST_HASH_RECIPE=shasum -a 256 docs/operations/p1.5-global-soak-contract.md docs/operations/p1.5-global-soak-scenarios.md | shasum -a 256
DEPENDENCY_ERROR_BASELINE=16
DEPENDENCY_ERROR_DELTA=0
COMPETITIVE_BENCH=FORBIDDEN
THROUGHPUT_CROSS_ARCH=FORBIDDEN
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
    --exclude 'docs/operations/p1.1-soak-evidence' \
    --exclude 'docs/operations/evidence/p1.2-soak' \
    --exclude 'docs/operations/evidence/p1.3a-soak' \
    --exclude 'docs/operations/evidence/p1.3b-soak' \
    --exclude 'docs/operations/evidence/p1.5-ws4/*/amd64' \
    --exclude 'docs/operations/evidence/p1.5-ws4/*/arm64' \
    "$ROOT/" "${host}:${workspace}/"
}

want_scenario() {
  local id="$1"
  [[ -z "$ONLY_SCENARIO" || "$ONLY_SCENARIO" == "$id" || "$ONLY_SCENARIO" == "all" ]]
}

phase_needed() {
  # Map which remote phases must run given --scenario filter.
  local phase="$1"
  if [[ -z "$ONLY_SCENARIO" || "$ONLY_SCENARIO" == "all" ]]; then
    return 0
  fi
  case "$ONLY_SCENARIO" in
    S01|S02|S08|S09) [[ "$phase" == "P11" ]] && return 0 ;;
    S03|S04|S11) [[ "$phase" == "P12" ]] && return 0 ;;
    S05|S06) [[ "$phase" == "P13A" ]] && return 0 ;;
    S07) [[ "$phase" == "P13B" ]] && return 0 ;;
    S10|S14) [[ "$phase" == "S10" ]] && return 0 ;;
    S15) [[ "$phase" == "S15" ]] && return 0 ;;
    S16) [[ "$phase" == "S16" ]] && return 0 ;;
    S12) [[ "$phase" == "P11" || "$phase" == "P12" ]] && return 0 ;;
    S13) [[ "$phase" == "P13A" || "$phase" == "P13B" || "$phase" == "S15" ]] && return 0 ;;
  esac
  return 1
}

run_remote_phase() {
  local host="$1" label="$2" arch="$3" workspace="$4" phase="$5"
  local local_dir="$ARTIFACT_DIR/$label"
  mkdir -p "$local_dir"
  local log="$ARTIFACT_DIR/orchestrator/${label}-${phase}.log"
  local rc=0
  local run_suffix="${LOCK_ID}-${phase}"
  echo "=== $label $phase on $host ===" | tee "$log"

  case "$phase" in
    P11)
      if ! ssh "${SSH_OPTS[@]}" "$host" \
        "source ~/.cargo/env 2>/dev/null || true; cd '$workspace'; \
         P1_1_SOAK_RUN_ID='$run_suffix' P1_1_SOAK_COMMIT='$HEAD' \
         bash scripts/soak/p1.1-correctness-stability-soak-remote.sh \
           --workspace '$workspace' --host-label '$label' --expected-arch '$arch' \
           --duration-sec '$D_P11' \
           --report-relpath 'docs/operations/evidence/p1.5-ws4/$LOCK_ID/$label/p11-report.md'" \
        >>"$log" 2>&1; then
        rc=$?
      fi
      mkdir -p "$local_dir/P11"
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${workspace}/docs/operations/p1.1-soak-evidence/${run_suffix}/${label}/" \
        "$local_dir/P11/" 2>/dev/null || true
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${workspace}/docs/operations/evidence/p1.5-ws4/${LOCK_ID}/${label}/p11-report.md" \
        "$local_dir/p11-report.md" 2>/dev/null || true
      ;;
    P12)
      local conc=16
      [[ "$label" == "arm64" ]] && conc=8
      if ! ssh "${SSH_OPTS[@]}" "$host" \
        "source ~/.cargo/env 2>/dev/null || true; cd '$workspace'; \
         P1_2_SOAK_RUN_ID='$run_suffix' P1_2_SOAK_LOCK_ID='$LOCK_ID' \
         bash scripts/soak/p1.2-fastcgi-wordpress-soak-remote.sh \
           --workspace '$workspace' --host-label '$label' --expected-arch '$arch' \
           --duration-sec '$D_P12' --warmup-sec 60 --concurrency '$conc' \
           --report-relpath 'docs/operations/evidence/p1.5-ws4/$LOCK_ID/$label/p12-report.md'" \
        >>"$log" 2>&1; then
        rc=$?
      fi
      mkdir -p "$local_dir/P12"
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${workspace}/docs/operations/evidence/p1.2-soak/${run_suffix}/${label}/" \
        "$local_dir/P12/" 2>/dev/null || true
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${workspace}/docs/operations/evidence/p1.5-ws4/${LOCK_ID}/${label}/p12-report.md" \
        "$local_dir/p12-report.md" 2>/dev/null || true
      ;;
    P13A)
      local conc=12
      [[ "$label" == "arm64" ]] && conc=6
      if ! ssh "${SSH_OPTS[@]}" "$host" \
        "source ~/.cargo/env 2>/dev/null || true; cd '$workspace'; \
         P1_3A_SOAK_RUN_ID='$run_suffix' P1_3A_SOAK_LOCK_ID='$LOCK_ID' \
         bash scripts/soak/p1.3a-proxy-tls-h2-soak-remote.sh \
           --workspace '$workspace' --host-label '$label' --expected-arch '$arch' \
           --duration-sec '$D_P13A' --warmup-sec 60 --concurrency '$conc' \
           --report-relpath 'docs/operations/evidence/p1.5-ws4/$LOCK_ID/$label/p13a-report.md'" \
        >>"$log" 2>&1; then
        rc=$?
      fi
      mkdir -p "$local_dir/P13A"
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${workspace}/docs/operations/evidence/p1.3a-soak/${run_suffix}/${label}/" \
        "$local_dir/P13A/" 2>/dev/null || true
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${workspace}/docs/operations/evidence/p1.5-ws4/${LOCK_ID}/${label}/p13a-report.md" \
        "$local_dir/p13a-report.md" 2>/dev/null || true
      ;;
    P13B)
      local conc=12
      [[ "$label" == "arm64" ]] && conc=6
      if ! ssh "${SSH_OPTS[@]}" "$host" \
        "source ~/.cargo/env 2>/dev/null || true; cd '$workspace'; \
         P1_3B_SOAK_RUN_ID='$run_suffix' P1_3B_SOAK_LOCK_ID='$LOCK_ID' \
         bash scripts/soak/p1.3b-http3-proxy-soak-remote.sh \
           --workspace '$workspace' --host-label '$label' --expected-arch '$arch' \
           --duration-sec '$D_P13B' --warmup-sec 60 --concurrency '$conc' \
           --report-relpath 'docs/operations/evidence/p1.5-ws4/$LOCK_ID/$label/p13b-report.md'" \
        >>"$log" 2>&1; then
        rc=$?
      fi
      mkdir -p "$local_dir/P13B"
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${workspace}/docs/operations/evidence/p1.3b-soak/${run_suffix}/${label}/" \
        "$local_dir/P13B/" 2>/dev/null || true
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${workspace}/docs/operations/evidence/p1.5-ws4/${LOCK_ID}/${label}/p13b-report.md" \
        "$local_dir/p13b-report.md" 2>/dev/null || true
      ;;
    S10|S15|S16)
      local thin_dur="$D_S10"
      [[ "$phase" == "S15" ]] && thin_dur="$D_S15"
      [[ "$phase" == "S16" ]] && thin_dur="$D_S16"
      local thin_art="$workspace/docs/operations/evidence/p1.5-ws4/$LOCK_ID/$label/thin"
      if ! ssh "${SSH_OPTS[@]}" "$host" \
        "source ~/.cargo/env 2>/dev/null || true; cd '$workspace'; \
         P15_WS4_S10_DURATION_SEC='$D_S10' P15_WS4_S15_DURATION_SEC='$D_S15' P15_WS4_S16_DURATION_SEC='$D_S16' \
         P15_WS4_SAMPLE_INTERVAL='$SAMPLE_INTERVAL' \
         bash scripts/soak/p15-ws4-thin-phases-remote.sh \
           --workspace '$workspace' --host-label '$label' --expected-arch '$arch' \
           --artifact-dir '$thin_art' --phase '$phase' --duration-sec '$thin_dur' \
           --sample-interval '$SAMPLE_INTERVAL'" \
        >>"$log" 2>&1; then
        rc=$?
      fi
      mkdir -p "$local_dir/thin"
      rsync -az -e "ssh ${SSH_OPTS[*]}" \
        "${host}:${thin_art}/" \
        "$local_dir/thin/" 2>/dev/null || true
      ;;
  esac

  echo "REMOTE_RC_${label}_${phase}=$rc" | tee -a "$log"
  # Fail-closed: phase exit 0 is insufficient if report/gates show FAIL.
  local report=""
  case "$phase" in
    P11) report="$local_dir/p11-report.md" ;;
    P12) report="$local_dir/p12-report.md" ;;
    P13A) report="$local_dir/p13a-report.md" ;;
    P13B) report="$local_dir/p13b-report.md" ;;
  esac
  if [[ -n "$report" && -f "$report" ]] && grep -q 'VERDICT = FAIL\|\*\*FAIL\*\*' "$report"; then
    echo "GATE_FAIL_${label}_${phase}=report_verdict" | tee -a "$log"
    rc=1
  fi
  if [[ -f "$local_dir/$phase/gates.csv" ]] && grep -q ',FAIL,' "$local_dir/$phase/gates.csv"; then
    echo "GATE_FAIL_${label}_${phase}=gates_csv" | tee -a "$log"
    rc=1
  fi
  if [[ "$phase" == "P13B" ]]; then
    if [[ -f "$local_dir/P13B/gates.csv" ]]; then
      if ! grep -q 'BOOT,PASS' "$local_dir/P13B/gates.csv" || ! grep -q 'H3_CLIENT,PASS' "$local_dir/P13B/gates.csv"; then
        echo "GATE_FAIL_${label}_${phase}=boot_or_h3_client" | tee -a "$log"
        rc=1
      fi
    else
      echo "GATE_FAIL_${label}_${phase}=missing_gates" | tee -a "$log"
      rc=1
    fi
  fi
  echo "$rc" >"$local_dir/rc-${phase}.txt"
  return "$rc"
}

run_arch() {
  local label="$1" host="$2" arch="$3" workspace="$4"
  local rc=0
  sync_tree "$host" "$workspace" | tee "$ARTIFACT_DIR/orchestrator/${label}-rsync.log"
  mkdir -p "$ARTIFACT_DIR/$label"

  local phases=()
  phase_needed P11 && phases+=(P11)
  phase_needed P12 && phases+=(P12)
  phase_needed P13A && phases+=(P13A)
  phase_needed P13B && phases+=(P13B)
  phase_needed S10 && phases+=(S10)
  phase_needed S15 && phases+=(S15)
  phase_needed S16 && phases+=(S16)

  # Default --all: all phases
  if [[ "$RUN_ALL" -eq 1 && -z "$ONLY_SCENARIO" ]]; then
    phases=(P11 P12 P13A P13B S10 S15 S16)
  fi

  local p
  for p in "${phases[@]}"; do
    if ! run_remote_phase "$host" "$label" "$arch" "$workspace" "$p"; then
      rc=1
    fi
  done

  # Aggregate scenario map for this arch
  python3 - "$ARTIFACT_DIR/$label" "$label" "$LOCK_ID" <<'PY'
import json, pathlib, sys
arch_dir = pathlib.Path(sys.argv[1])
label = sys.argv[2]
lock = sys.argv[3]
rcs = {}
for p in arch_dir.glob("rc-*.txt"):
    rcs[p.stem[3:]] = int(p.read_text().strip() or "1")
# Map scenarios to phase RCs
mapping = {
  "S01": "P11", "S02": "P11",
  "S03": "P12", "S04": "P12", "S11": "P12",
  "S05": "P13A", "S06": "P13A",
  "S07": "P13B",
  "S08": ("P11", "P12", "P13A", "P13B"),
  "S09": ("P11", "P12", "P13A", "P13B"),
  "S10": "S10", "S14": "S10",
  "S15": "S15", "S16": "S16",
  "S12": ("P11", "P12"),
  "S13": ("P13A", "P13B", "S15"),
}
scenarios = {}
fail = 0
for sid, src in mapping.items():
    if isinstance(src, tuple):
        if not all(x in rcs for x in src):
            # Phase not executed in this run (filtered --scenario) — skip.
            continue
        codes = [rcs[x] for x in src]
        ok = all(c == 0 for c in codes)
        scenarios[sid] = {"phases": list(src), "rcs": codes, "verdict": "PASS" if ok else "FAIL"}
        if not ok: fail += 1
    else:
        if src not in rcs:
            continue
        code = rcs[src]
        scenarios[sid] = {"phase": src, "rc": code, "verdict": "PASS" if code == 0 else "FAIL"}
        if code != 0: fail += 1
out = {
  "lock_id": lock,
  "arch_label": label,
  "phase_rcs": rcs,
  "scenarios": scenarios,
  "verdict": "PASS" if fail == 0 and rcs else "FAIL",
}
(arch_dir / "summary.json").write_text(json.dumps(out, indent=2) + "\n")
(arch_dir / "summary.txt").write_text(
  f"LOCK={lock}\nARCH={label}\nVERDICT={out['verdict']}\n" +
  "\n".join(f"{k}={v['verdict']}" for k, v in scenarios.items()) + "\n"
)
print(out["verdict"])
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
  if ! run_arch "$label" "$host" "$arch" "$workspace"; then
    RC=1
  fi
done

# Dual-arch reconciliation skeleton
python3 - "$ARTIFACT_DIR" "$LOCK_ID" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
lock = sys.argv[2]
amd = root / "amd64" / "summary.json"
arm = root / "arm64" / "summary.json"
def load(p):
  return json.loads(p.read_text()) if p.exists() else None
a, b = load(amd), load(arm)
verdict = "INCOMPLETE"
notes = []
if a and b:
  if a.get("verdict") == "PASS" and b.get("verdict") == "PASS":
    verdict = "PASS"
  else:
    verdict = "FAIL"
    notes.append("one or both arches FAIL")
elif a or b:
  verdict = "PARTIAL"
  notes.append("missing one arch")
out = {
  "lock_id": lock,
  "amd64": a,
  "arm64": b,
  "THROUGHPUT_CROSS_ARCH": "FORBIDDEN",
  "DUAL_ARCH_WS4_RECONCILIATION": verdict,
  "notes": notes,
}
(root / "summary.json").write_text(json.dumps(out, indent=2) + "\n")
(root / "summary.txt").write_text(
  f"LOCK={lock}\nDUAL_ARCH_WS4_RECONCILIATION={verdict}\n"
  f"AMD64={a and a.get('verdict')}\nARM64={b and b.get('verdict')}\n"
)
print(verdict)
PY

if [[ "$FORMAT" == "json" ]]; then
  cat "$ARTIFACT_DIR/summary.json"
else
  cat "$ARTIFACT_DIR/summary.txt"
fi

echo "P15_WS4_LOCK=$LOCK_ID ARTIFACT_DIR=$ARTIFACT_DIR RC=$RC"
exit "$RC"
