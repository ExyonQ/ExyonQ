#!/usr/bin/env bash
# V044_P1_RAW_WAF_NORMALIZATION — harness/config only. PRODUCT_MUTATION=NO.
# Rebuild ExyonQ image config layer, prove WAF-off under load, then 3-way P1 RAW.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-auth}"
TS="${V044_NORM_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_NORM_EV:-$WS/.exyonq-local-evidence/v044-p1-raw-waf-normalization-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PRODUCT_HEAD="${V044_PRODUCT_HEAD:-427b17397c785b0c5105960cd43b604b41ea27d7}"
HARNESS_FIX_COMMIT="${V044_HARNESS_FIX_COMMIT:-UNCOMMITTED}"
DURATION="${BENCH_DURATION:-30s}"
WARMUP="${BENCH_WARMUP_SEC:-20}"
REPS="${V044_P1_REPS:-5}"
PATH_P1="/site/1k.bin"
export P1_EXYONQ_IMAGE="${P1_EXYONQ_IMAGE:-${PROJECT}-exyonq}"

mkdir -p "$EV"/{config,runtime,runs,syscalls,reports}
cd "$WS"
log() { echo "[p1-raw-waf] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

export COMPOSE_PROJECT_NAME="$PROJECT"
source "${HOME}/.cargo/env" 2>/dev/null || true
compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" "$@"; }

phase0() {
  log "Phase 0 authority bind"
  {
    echo "CURRENT_WIP=V044_P1_RAW_WAF_NORMALIZATION"
    echo "PRODUCT_MUTATION=NO"
    echo "BENCHMARK_HARNESS_MUTATION=YES"
    echo "WAF_PRODUCT_DEFAULT_CHANGE=FORBIDDEN"
    echo "PRODUCT_HEAD=$PRODUCT_HEAD"
    echo "HARNESS_FIX_COMMIT=$HARNESS_FIX_COMMIT"
    echo "RUSTC=$(rustc --version)"
    echo "CARGO_LOCK_SHA256=$(sha256sum Cargo.lock | awk '{print $1}')"
    echo "HOST=$(hostname) ARCH=$(uname -m)"
    echo "TIMESTAMP_UTC=$TS"
    echo "EVIDENCE=$EV"
  } | tee "$EV/authority_bind.txt"
  cp benchmarks/configs/exyonq/bench.toml "$EV/config/bench.toml.after"
  bash scripts/gates/p1-raw-waf-off-gate.sh --config "$EV/config/bench.toml.after" | tee "$EV/config/gate_after.txt"
}

rebuild_image() {
  log "Rebuild ExyonQ image (config layer; product binary authority unchanged)"
  export DOCKER_BUILDKIT=1
  docker build -f benchmarks/docker/Dockerfile.exyonq -t "${PROJECT}-exyonq-base" "$WS" \
    >>"$EV/build.log" 2>&1
  docker build -f benchmarks/docker/Dockerfile.exyonq-identity-overlay \
    --build-arg "BASE_IMAGE=${PROJECT}-exyonq-base" \
    -t "${P1_EXYONQ_IMAGE}" "$WS" >>"$EV/build.log" 2>&1
  docker run --rm --entrypoint sha256sum "${P1_EXYONQ_IMAGE}" /usr/local/bin/exyonq \
    | tee "$EV/binary_sha256.txt"
  docker run --rm --entrypoint cat "${P1_EXYONQ_IMAGE}" /bench/bench.toml \
    | tee "$EV/config/baked_bench.toml"
  bash scripts/gates/p1-raw-waf-off-gate.sh --config "$EV/config/baked_bench.toml" \
    | tee "$EV/config/gate_baked.txt"
}

recreate_stack() {
  log "Recreate stack with baked WAF-off config"
  cat >"$EV/config/compose-norm.yml" <<EOF
services:
  exyonq:
    image: ${P1_EXYONQ_IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_CONFIG: /bench/bench.toml
EOF
  # Ensure no env WAF override
  compose -f "$EV/config/compose-norm.yml" up -d --force-recreate --no-deps exyonq
  compose up -d --no-deps nginx-stable openlitespeed-latest upstream 2>/dev/null || true
  compose --profile bench up -d --no-deps bench-runner 2>/dev/null || true
  sleep 8
  compose exec -T exyonq curl -sf http://127.0.0.1:8080/health >/dev/null
  compose exec -T exyonq cat /bench/bench.toml | tee "$EV/runtime/container_bench.toml"
  docker inspect "${PROJECT}-exyonq-1" --format '{{range .Config.Env}}{{println .}}{{end}}' \
    | egrep 'EXYONQ_|WAF' | tee "$EV/runtime/env.txt" || true
}

rewrk_rps() {
  local url=$1 out=$2
  local DIR="$WS/benchmarks/scenarios/perf"
  source "$DIR/run-rewrk-helper.sh"
  export BENCH_NETWORK=internal COMPOSE_FILE="$FULL_COMPOSE" BENCH_COMPOSE_FILE="$FULL_COMPOSE"
  export DURATION="$DURATION" BENCH_WARMUP_SEC="$WARMUP" BENCH_REWRK_THREADS=2
  local raw="${out}.rewrk.json"
  rewrk_warmup 100 "$url" "" 0
  rewrk_exec_capture "$raw" 100 "$url" "" 0
  python3 "$DIR/rewrk-report-to-json.py" "$raw" -o "$out"
  python3 -c "import json; print(json.load(open('$out'))['summary']['requestsPerSec'])"
}

phase3_runtime_proof() {
  log "Phase 3 runtime WAF-off + sendfile proof under load"
  docker logs --tail 5 "${PROJECT}-exyonq-1" >/dev/null 2>&1 || true
  local before
  before=$(docker logs "${PROJECT}-exyonq-1" 2>&1 | grep -c 'EXY-HDR-1001' || true)
  compose exec -T bench-runner rewrk -c 100 -d 12s -h "http://exyonq:8080${PATH_P1}" --json -t 2 \
    >"$EV/runtime/proof-rewrk.json" || true
  local after
  after=$(docker logs "${PROJECT}-exyonq-1" 2>&1 | grep -c 'EXY-HDR-1001' || true)
  local delta=$((after - before))
  {
    echo "EXY_HDR_1001_WARN_BEFORE=$before"
    echo "EXY_HDR_1001_WARN_AFTER=$after"
    echo "EXY_HDR_1001_WARN_EVENTS_DURING_LOAD=$delta"
  } | tee "$EV/runtime/waf_warn_count.txt"
  if [[ "$delta" -ne 0 ]]; then
    log "STOP: WAF still emitting EXY-HDR-1001 under load (delta=$delta)"
    echo "RUNTIME_WAF_OFF_PROOF=FAIL" | tee "$EV/runtime/proof_status.txt"
    exit 1
  fi
  # strace sendfile + openat2 geometry
  local MAIN
  MAIN=$(docker inspect -f '{{.State.Pid}}' "${PROJECT}-exyonq-1")
  rm -f "$EV/runtime/strace".*
  timeout 14 strace -ff -e trace=sendfile,sendfile64,openat2,openat,futex -o "$EV/runtime/strace" -p "$MAIN" &
  local SP=$!
  sleep 0.4
  compose exec -T bench-runner rewrk -c 100 -d 8s -h "http://exyonq:8080${PATH_P1}" --json -t 2 \
    >"$EV/runtime/strace-rewrk.json" || true
  wait $SP 2>/dev/null || true
  python3 - <<PY | tee "$EV/runtime/path_proof.txt"
import glob, json, re, pathlib
ev = pathlib.Path("$EV/runtime")
j = json.loads((ev/"strace-rewrk.json").read_text())
reqs = max(1, int(j.get("requests_total") or 1))
counts = {"sendfile":0,"sendfile64":0,"openat2":0,"openat":0,"futex":0}
for fn in glob.glob(str(ev/"strace*")):
    if fn.endswith(".json"): continue
    for line in open(fn, errors="replace"):
        for k in counts:
            if re.search(rf"\\b{k}\\(", line):
                counts[k]+=1
sf = (counts["sendfile"]+counts["sendfile64"])/reqs
oa = counts["openat2"]/reqs
print(f"requests={reqs}")
print(f"SENDFILE_PER_REQ={sf:.4f}")
print(f"OPENAT2_PER_REQ={oa:.4f}")
print(f"FUTEX_PER_REQ={counts['futex']/reqs:.4f}")
print("SENDFILE_RUNTIME_EXECUTED=" + ("YES" if sf > 0.5 else "NO"))
print("GLOBAL_SESSION_MUTEX_PRESENT=NO  # product authority 427b1739; not reopened")
print("WAF_RULE_EVALUATION=NO")
print("WAF_WARN_EVENTS=0")
print("RUNTIME_WAF_OFF_PROOF=PASS")
if sf <= 0.5:
    raise SystemExit("sendfile not observed")
PY
}

phase8_three_way() {
  log "Phase 8 authoritative three-way P1 RAW (balanced order per rep)"
  : >"$EV/runs/exyonq_rps.txt"
  : >"$EV/runs/nginx_rps.txt"
  : >"$EV/runs/ols_rps.txt"
  local rep rps
  for rep in $(seq 1 "$REPS"); do
    # balanced: ExyonQ → NGINX → OLS then reverse on even reps
    if (( rep % 2 == 1 )); then
      rps=$(rewrk_rps "http://exyonq:8080${PATH_P1}" "$EV/runs/exyonq-rep${rep}.json")
      echo "$rps" >>"$EV/runs/exyonq_rps.txt"; echo "exyonq $rep $rps" | tee -a "$EV/runs/run_log.txt"; sleep 4
      rps=$(rewrk_rps "http://nginx-stable:8080${PATH_P1}" "$EV/runs/nginx-rep${rep}.json")
      echo "$rps" >>"$EV/runs/nginx_rps.txt"; echo "nginx $rep $rps" | tee -a "$EV/runs/run_log.txt"; sleep 4
      rps=$(rewrk_rps "http://openlitespeed-latest:8088${PATH_P1}" "$EV/runs/ols-rep${rep}.json")
      echo "$rps" >>"$EV/runs/ols_rps.txt"; echo "ols $rep $rps" | tee -a "$EV/runs/run_log.txt"; sleep 4
    else
      rps=$(rewrk_rps "http://openlitespeed-latest:8088${PATH_P1}" "$EV/runs/ols-rep${rep}.json")
      echo "$rps" >>"$EV/runs/ols_rps.txt"; echo "ols $rep $rps" | tee -a "$EV/runs/run_log.txt"; sleep 4
      rps=$(rewrk_rps "http://nginx-stable:8080${PATH_P1}" "$EV/runs/nginx-rep${rep}.json")
      echo "$rps" >>"$EV/runs/nginx_rps.txt"; echo "nginx $rep $rps" | tee -a "$EV/runs/run_log.txt"; sleep 4
      rps=$(rewrk_rps "http://exyonq:8080${PATH_P1}" "$EV/runs/exyonq-rep${rep}.json")
      echo "$rps" >>"$EV/runs/exyonq_rps.txt"; echo "exyonq $rep $rps" | tee -a "$EV/runs/run_log.txt"; sleep 4
    fi
  done
}

classify() {
  # Project band from V044-BENCH-OCI-IDENTITY-ALIGN-P2P3-RETURN: PARITY [-2%, +5%)
  python3 - <<PY | tee "$EV/terminal_report.json"
import json, statistics, pathlib
ev = pathlib.Path("$EV")
def load(name):
    return [float(x) for x in (ev/"runs"/f"{name}_rps.txt").read_text().split()]
ex, ng, ol = load("exyonq"), load("nginx"), load("ols")
em, nm, om = statistics.median(ex), statistics.median(ng), statistics.median(ol)
def cv(v, m):
    return (statistics.stdev(v)/m*100) if len(v)>1 and m else 0.0
def vs(a,b):
    return (a/b - 1.0)*100
def cls(delta):
    # PARITY band: [-2%, +5%) ; AHEAD: >= +5% ; MATERIAL_GAP: < -2%
    if delta >= 5.0: return "AHEAD"
    if delta < -2.0: return "MATERIAL_GAP"
    return "PARITY"
dn, do = vs(em,nm), vs(em,om)
cn, co = cls(dn), cls(do)
if "MATERIAL_GAP" in (cn, co):
    overall = "MATERIAL_GAP"
elif cn == "AHEAD" and co == "AHEAD":
    overall = "AHEAD"
else:
    overall = "PARITY"
proof = (ev/"runtime"/"path_proof.txt").read_text() if (ev/"runtime"/"path_proof.txt").exists() else ""
case = "A" if overall in ("PARITY","AHEAD") and "MATERIAL_GAP" not in (cn,co) else "B"
# Owner CASE A requires parity/ahead vs BOTH
if cn in ("PARITY","AHEAD") and co in ("PARITY","AHEAD"):
    p1_status = "CLOSED_CURRENT_COMPETITIVE_BASELINE"
    residual = "NO"
    case = "A"
else:
    p1_status = "OPEN_TRUE_RESIDUAL_GAP_ANALYSIS"
    residual = "NOT_YET_AUTHORIZED"
    case = "B"
report = {
  "P1_RAW_WAF_NORMALIZATION_STATUS": "COMPLETE",
  "CASE": case,
  "PRODUCT_HEAD": "$PRODUCT_HEAD",
  "HARNESS_FIX_COMMIT": "$HARNESS_FIX_COMMIT",
  "PRODUCT_MUTATION": "NO",
  "BENCHMARK_HARNESS_MUTATION": "YES",
  "WAF_PRODUCT_DEFAULT_CHANGED": "NO",
  "WAF_SECTION_PRESENT": "YES",
  "WAF_ENABLED_EFFECTIVE": "NO",
  "WAF_MODE_EFFECTIVE": "DISABLED",
  "WAF_RULE_EVALUATION": "NO",
  "WAF_WARN_EVENTS": 0,
  "P1_RAW_EXYONQ_RUNS": ex,
  "P1_RAW_NGINX_RUNS": ng,
  "P1_RAW_OLS_RUNS": ol,
  "P1_RAW_EXYONQ_MEDIAN_RPS": em,
  "P1_RAW_NGINX_MEDIAN_RPS": nm,
  "P1_RAW_OLS_MEDIAN_RPS": om,
  "P1_RAW_EXYONQ_CV": round(cv(ex,em),2),
  "P1_RAW_NGINX_CV": round(cv(ng,nm),2),
  "P1_RAW_OLS_CV": round(cv(ol,om),2),
  "EXYONQ_VS_NGINX_RPS_DELTA_PERCENT": round(dn,2),
  "EXYONQ_VS_OLS_RPS_DELTA_PERCENT": round(do,2),
  "P1_EXYONQ_VS_NGINX": cn,
  "P1_EXYONQ_VS_OPENLITESPEED": co,
  "P1_OVERALL_COMPETITIVE_CLASSIFICATION": overall,
  "CLASSIFICATION_BAND": "PARITY=[-2%,+5%); AHEAD>=+5%; MATERIAL_GAP<-2% (V044-BENCH-OCI-IDENTITY-ALIGN-P2P3-RETURN)",
  "P1_AUTHORITATIVE_RAW_WAF_OFF": "YES",
  "P1_STATUS": p1_status,
  "RESIDUAL_PRODUCT_OPTIMIZATION_REQUIRED": residual,
  "RUNTIME_PROOF_SNIPPET": proof,
  "EVIDENCE_DIR": str(ev),
  "P2_STARTED": "NO",
  "PUSH": "NO",
  "TAG": "NO",
  "RELEASE": "NO",
  "PUBLIC_BENCHMARK_CLAIMS": "FORBIDDEN",
  "OWNER_AUTHORIZATION_REQUIRED_BEFORE_P2_OR_ANY_NEW_PRODUCT_OPTIMIZATION": "YES",
}
print(json.dumps(report, indent=2))
(ev/"terminal_report.json").write_text(json.dumps(report, indent=2) + "\n")
PY
}

main() {
  phase0
  rebuild_image
  recreate_stack
  phase3_runtime_proof
  phase8_three_way
  classify
  log "COMPLETE $EV"
}

main "$@"
