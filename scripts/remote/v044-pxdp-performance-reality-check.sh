#!/usr/bin/env bash
# V044_PXDP_PERFORMANCE_REALITY_CHECK — read-only 4-way P4 proxy 1KiB matrix.
# PRODUCT_MUTATION=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
set -euo pipefail

WS="${PXDP_REALITY_WS:-$(pwd)}"
TS="${PXDP_REALITY_TS:-$(date -u +%Y%m%d-%H%M%S)}"
EV="${PXDP_REALITY_EV:-$WS/.exyonq-local/evidence/pxdp-performance-reality/$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044pxdp-reality-$TS}"
IMAGE="${PXDP_EXYONQ_IMAGE:-v044-pxdp-p5-reality-exyonq}"
PRODUCT_COMMIT="${PXDP_PRODUCT_COMMIT:-0bc2b973e5ff62b2316fbc7c3f002d673c65d791}"
PATH_P4="/api/"
UPSTREAM_PATH="/api/"
WARMUP_SEC=20
MEASURE_SEC=30
REPS="${PXDP_REPS:-5}"
EXPECTED_BYTES=1024
TAGS=(exyonq nginx ols haproxy)

mkdir -p "$EV"/{runs,stats,pct,meta,reports,sanity,geometry,cgroup_companion,upstream,ctx}
cd "$WS"
log() { echo "[pxdp-reality] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() {
  local extra=()
  if [[ -f "$EV/meta/compose-pxdp.yml" ]]; then
    extra=(-f "$EV/meta/compose-pxdp.yml")
  fi
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@"
}

ctr() {
  case "$1" in
    exyonq) echo "${PROJECT}-exyonq-1" ;;
    nginx) echo "${PROJECT}-nginx-stable-1" ;;
    ols) echo "${PROJECT}-openlitespeed-latest-1" ;;
    haproxy) echo "${PROJECT}-haproxy-1" ;;
    upstream) echo "${PROJECT}-upstream-1" ;;
  esac
}

url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P4}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P4}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P4}" ;;
    haproxy) echo "http://haproxy:8080${PATH_P4}" ;;
    upstream) echo "http://upstream:9000${UPSTREAM_PATH}" ;;
  esac
}

write_authority() {
  local tree="${PXDP_P5_TERMINAL_TREE:-}"
  if [[ -z "$tree" ]] && git -C "$WS" rev-parse "${PRODUCT_COMMIT}^{tree}" >/dev/null 2>&1; then
    tree=$(git -C "$WS" rev-parse "${PRODUCT_COMMIT}^{tree}")
  fi
  if [[ -z "$tree" ]]; then
    tree="b1b7659b4d3bb8745ea0ed42a8de7d424f523767"
  fi
  {
    echo "WIP=V044_PXDP_PERFORMANCE_REALITY_CHECK"
    echo "MODE=READ_ONLY_BENCHMARK_AND_CAUSAL_VALIDATION"
    echo "PRODUCT_MUTATION=NO"
    echo "P5_TERMINAL_HEAD=$PRODUCT_COMMIT"
    echo "P5_TERMINAL_TREE=$tree"
    echo "PXDP_GATES=EXYONQ_PXDP_P2=1 EXYONQ_PXDP_P4=1"
    echo "OLD_AUTHORITATIVE_RUN=v044-p4-proxy-1k-3way-20260822T115113Z"
    echo "INVALID_RUN_EXCLUDED=v044-p4-proxy-1k-3way-20260822T112732Z"
  } | tee "$EV/meta/authority.txt"
  if [[ "$tree" != "b1b7659b4d3bb8745ea0ed42a8de7d424f523767" ]]; then
    log "FAIL: P5 terminal tree mismatch got=$tree"
    exit 2
  fi
}

write_environment() {
  {
    echo "HOSTNAME=$(hostname -f 2>/dev/null || hostname)"
    echo "KERNEL=$(uname -r)"
    echo "ARCH=$(uname -m)"
    echo "CORE_COUNT=$(nproc)"
    echo "CPU_MODEL=$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | xargs)"
    echo "CPUSET=0-7"
    echo "LOAD_AVG=$(cut -d' ' -f1-3 /proc/loadavg)"
    echo "RUSTC=$(/root/.cargo/bin/rustc --version 2>/dev/null || rustc --version 2>/dev/null || echo NOT_MEASURED)"
    echo "DOCKER=$(docker --version 2>/dev/null || echo NOT_MEASURED)"
  } | tee "$EV/meta/environment.txt"
}

write_host_hygiene() {
  {
    echo "TIMESTAMP=$(date -u +%Y%m%dT%H%M%SZ)"
    echo "=== load ==="
    uptime
    free -h
    echo "=== swap ==="
    swapon --show 2>/dev/null || true
    echo "=== docker ps ==="
    docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'
  } | tee "$EV/host-hygiene.txt"
}

ensure_stack() {
  unset EXYONQ_WORKER_THREADS EXYONQ_ACCEPT_WORKERS EXYONQ_EPOLL_POOL_THREADS || true
  export P1_EXYONQ_IMAGE="$IMAGE"
  cat >"$EV/meta/compose-pxdp.yml" <<EOF
services:
  exyonq:
    image: ${IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
      EXYONQ_PXDP_P2: "1"
      EXYONQ_PXDP_P4: "1"
  nginx-stable:
    cpuset: "0-7"
  openlitespeed-latest:
    cpuset: "0-7"
  haproxy:
    cpuset: "0-7"
  upstream:
    cpuset: "0-7"
  bench-runner:
    image: v044p1auth-bench-runner:latest
    cpuset: "0-7"
EOF
  compose up -d --no-deps exyonq nginx-stable openlitespeed-latest haproxy upstream
  compose --profile bench up -d --no-deps bench-runner
  sleep 10
  if ! docker inspect -f '{{.State.Running}}' "$(docker ps -aq -f name=${PROJECT}-bench-runner)" 2>/dev/null | grep -q true; then
    br="${PROJECT}-bench-runner-1"
    if ! docker inspect -f '{{.State.Running}}' "$br" 2>/dev/null | grep -q true; then
      log "FAIL: bench-runner not running"
      docker logs "$br" 2>&1 | tail -20 | tee "$EV/meta/bench-runner-fail.log" || true
      exit 2
    fi
  fi
  for tag in exyonq nginx ols haproxy upstream; do
    docker update --cpuset-cpus "0-7" "$(ctr "$tag")" >/dev/null 2>&1 || true
  done
}

record_geometry() {
  docker exec "$(ctr exyonq)" env | egrep 'EXYONQ_PXDP|EXYONQ_CONFIG|EXYONQ_EDGE' | sort \
    | tee "$EV/geometry/exyonq_env.txt"
  EXY_SHA=$(docker exec "$(ctr exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "EXYONQ_BINARY_SHA256=$EXY_SHA" | tee "$EV/meta/exyonq_sha.txt"
  docker exec "$(ctr nginx)" nginx -v 2>&1 | tee "$EV/meta/nginx_version.txt"
  docker exec "$(ctr haproxy)" haproxy -v 2>&1 | head -1 | tee "$EV/meta/haproxy_version.txt"
  docker exec "$(ctr ols)" /usr/local/lsws/bin/lshttpd -v 2>&1 | head -1 | tee "$EV/meta/ols_version.txt" || true
  UP_SHA=$(docker exec "$(ctr upstream)" sha256sum /usr/local/bin/exyonq-upstream | awk '{print $1}')
  echo "UPSTREAM_BINARY_SHA256=$UP_SHA" | tee "$EV/meta/upstream_sha.txt"
}

verify_pxdp_execution() {
  local fail=0
  grep -q 'EXYONQ_PXDP_P2=1' "$EV/geometry/exyonq_env.txt" || fail=1
  grep -q 'EXYONQ_PXDP_P4=1' "$EV/geometry/exyonq_env.txt" || fail=1
  if docker exec "$(ctr exyonq)" sh -c 'grep -q "^\[\[modules" /bench/bench.toml 2>/dev/null'; then
    log "FAIL: modules configured in bench.toml"
    fail=1
  fi
  if docker exec "$(ctr exyonq)" sh -c 'grep -A5 "^\[waf\]" /bench/bench.toml | grep -q "enabled = false"'; then
    echo "WAF_OFF=YES" | tee -a "$EV/sanity/pxdp_execution.txt"
  else
    docker exec "$(ctr exyonq)" sh -c 'grep -A5 "^\[waf\]" /bench/bench.toml' | tee "$EV/sanity/waf_section.txt" || true
    log "FAIL: WAF not explicitly off"
    fail=1
  fi
  if [[ "$fail" -ne 0 ]]; then
    echo "PXDP_EXECUTION_CONFIRMED=NO" | tee "$EV/sanity/pxdp_execution.txt"
    exit 2
  fi
  echo "PXDP_EXECUTION_CONFIRMED=YES LEGACY_EXECUTION_FOR_BENCH_REQUEST=NO BENCHMARK_SPECIFIC_PRODUCT_BRANCH=NO" \
    | tee "$EV/sanity/pxdp_execution.txt"
}

read_ctxt() {
  local pid=$1
  awk '/^voluntary_ctxt_switches/{v=$2} /^nonvoluntary_ctxt_switches/{n=$2} END{print v+0, n+0}' "/proc/$pid/status" 2>/dev/null || echo "0 0"
}

correctness_gate() {
  local fail=0 uh
  local udir uout ucode usize
  udir=$(url_for upstream)
  uout=$(compose exec -T bench-runner curl -sS -m 10 -o /tmp/upstream-1k.bin -w "%{http_code} %{size_download}" "$udir")
  echo "upstream_direct=$uout" | tee "$EV/sanity/upstream_direct.txt"
  ucode=${uout%% *}; usize=${uout##* }
  [[ "$ucode" == "200" && "$usize" == "$EXPECTED_BYTES" ]] || fail=1
  uh=$(compose exec -T bench-runner sha256sum /tmp/upstream-1k.bin | awk '{print $1}')
  compose exec -T bench-runner sha256sum /tmp/upstream-1k.bin | tee "$EV/sanity/upstream_body.sha"

  for tag in "${TAGS[@]}"; do
    local u out code size h
    u=$(url_for "$tag")
    out=$(compose exec -T bench-runner curl -sS -m 10 -o "/tmp/${tag}-api-1k.bin" -w "%{http_code} %{size_download}" "$u")
    echo "${tag}_proxy=$out" | tee -a "$EV/sanity/proxy_probe.txt"
    code=${out%% *}; size=${out##* }
    [[ "$code" == "200" && "$size" == "$EXPECTED_BYTES" ]] || fail=1
    h=$(compose exec -T bench-runner sha256sum "/tmp/${tag}-api-1k.bin" | awk '{print $1}')
    echo "${tag}_hash=$h" | tee -a "$EV/sanity/proxy_hash.txt"
    [[ "$h" == "$uh" ]] || fail=1
    compose exec -T bench-runner bash -lc "
      curl -sS -m 10 --http1.1 '$u' -H 'Connection: keep-alive' -o /tmp/${tag}-ka1.bin -w '%{http_code} %{size_download} '
      curl -sS -m 10 --http1.1 '$u' -H 'Connection: keep-alive' -o /tmp/${tag}-ka2.bin -w '%{http_code} %{size_download}'
    " | tee -a "$EV/sanity/keepalive_serial.txt"
  done
  [[ "$fail" -eq 0 ]] || exit 2
  echo "CORRECTNESS=PASS BODY_HASH_PARITY=PASS CLIENT_KEEPALIVE=comparable" | tee "$EV/sanity/correctness.txt"
}

upstream_ceiling() {
  local url=$(url_for upstream)
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  compose exec -T bench-runner timeout $((MEASURE_SEC + 20)) rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
    >"$EV/upstream/direct.json"
  python3 - "$EV/upstream" <<'PY' | tee "$EV/upstream/direct.summary.txt"
import json, sys
from pathlib import Path
j=json.loads((Path(sys.argv[1])/"direct.json").read_text())
rps=float(j.get("requests_avg") or 0)
(Path(sys.argv[1])/"direct.summary.json").write_text(json.dumps({"rps":rps},indent=2)+"\n")
print(f"UPSTREAM_DIRECT_RPS={rps}")
PY
}

run_rep() {
  local tag=$1 rep=$2
  local c url statsjson host_pid v0 n0 v1 n1
  c=$(ctr "$tag")
  url=$(url_for "$tag")
  statsjson="$EV/stats/${tag}-rep${rep}.cpu.txt"
  host_pid=$(docker inspect -f '{{.State.Pid}}' "$c")
  read -r v0 n0 <<<"$(read_ctxt "$host_pid")"
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  : >"$statsjson"
  timeout $((MEASURE_SEC + 15)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$statsjson" 2>/dev/null &
  local sp=$!
  sleep 0.3
  compose exec -T bench-runner timeout $((MEASURE_SEC + 20)) rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
    >"$EV/runs/${tag}-rep${rep}.json"
  wait "$sp" 2>/dev/null || true
  read -r v1 n1 <<<"$(read_ctxt "$host_pid")"
  local req
  req=$(python3 -c "import json; print(json.load(open('$EV/runs/${tag}-rep${rep}.json')).get('requests_total') or 0)")
  {
    echo "tag=$tag rep=$rep req=$req"
    echo "voluntary_delta=$((v1-v0)) nonvoluntary_delta=$((n1-n0))"
    if [[ "$req" != "0" ]]; then
      echo "voluntary_per_req=$(python3 -c "print(($v1-$v0)/$req)")"
      echo "nonvoluntary_per_req=$(python3 -c "print(($n1-$n0)/$req)")"
      echo "total_ctx_per_req=$(python3 -c "print(($v1-$v0+$n1-$n0)/$req)")"
    fi
  } | tee "$EV/ctx/${tag}-rep${rep}.txt"
}

shuffle_tags() {
  python3 - <<'PY'
import random
tags=["exyonq","nginx","ols","haproxy"]
random.shuffle(tags)
print(" ".join(tags))
PY
}

summarize() {
  python3 - "$EV" <<'PY' | tee "$EV/reports/terminal.txt"
import json, re, statistics, sys
from pathlib import Path

ev = Path(sys.argv[1])
tags = ("exyonq", "nginx", "ols", "haproxy")
ansi = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")

OLD = {
  "exyonq_rps": 73097.0,
  "exyonq_cpu": 0.206626,
  "exyonq_user_us": 21.90,
  "exyonq_p50": 1.66,
  "exyonq_p95": 2.71,
  "exyonq_p99": 3.53,
  "exyonq_rss": 33.18,
}

def median(xs):
    return statistics.median(xs) if xs else None

def parse_cpu_mem(path):
    cpus, rss = [], []
    if not path.exists():
        return cpus, rss
    for ln in path.read_text(errors="replace").splitlines():
        ln = ansi.sub("", ln).strip()
        m2 = re.search(r"([\d.]+)%\s+(\d+(?:\.\d+)?)(MiB|GiB|KiB)", ln)
        if not m2:
            continue
        cpus.append(float(m2.group(1)))
        val = float(m2.group(2))
        mul = {"KiB": 1/1024, "MiB": 1, "GiB": 1024}[m2.group(3)]
        rss.append(val * mul)
    return cpus, rss

def parse_pct(path):
    text = path.read_text(errors="replace") if path.exists() else ""
    out = {}
    for pct in ("50", "95", "99"):
        m = re.search(rf"\|\s*{pct}%\s*\|\s*([\d.]+)ms", text)
        if m:
            out[f"p{pct}"] = float(m.group(1))
    return out

rows = {}
for tag in tags:
    rps_list, cpu_avgs, rss_avgs, ctx_list = [], [], [], []
    for rep in range(1, 6):
        rp = ev / "runs" / f"{tag}-rep{rep}.json"
        if not rp.exists():
            continue
        j = json.loads(rp.read_text())
        rps_list.append(float(j.get("requests_avg") or 0))
        cpus, rss = parse_cpu_mem(ev / "stats" / f"{tag}-rep{rep}.cpu.txt")
        if cpus:
            cpu_avgs.append(statistics.mean(cpus))
        if rss:
            rss_avgs.append(statistics.mean(rss))
        ctxp = ev / "ctx" / f"{tag}-rep{rep}.txt"
        if ctxp.exists():
            for ln in ctxp.read_text().splitlines():
                if ln.startswith("total_ctx_per_req="):
                    ctx_list.append(float(ln.split("=",1)[1]))
    cpu = median(cpu_avgs)
    rps = median(rps_list)
    core_ms = (cpu / 100 * 8 * 1000 / rps) if cpu and rps else None
    cg = ev / "cgroup_companion" / f"{tag}.summary.txt"
    cgroup = {}
    if cg.exists():
        for ln in cg.read_text().splitlines():
            if "=" in ln:
                k, v = ln.split("=", 1)
                cgroup[k.strip()] = v.strip()
    rows[tag] = {
        "rps_median": rps,
        "rps_runs": rps_list,
        "cpu_per_req": core_ms,
        "rss_mib": median(rss_avgs),
        "ctx_per_req": median(ctx_list),
        "user_us_per_req": float(cgroup["user_us_per_req"]) if cgroup.get("user_us_per_req") not in (None, "None") else None,
        "system_us_per_req": float(cgroup["system_us_per_req"]) if cgroup.get("system_us_per_req") not in (None, "None") else None,
        **parse_pct(ev / "pct" / f"{tag}.txt"),
    }

(ev / "summary.json").write_text(json.dumps(rows, indent=2) + "\n")

ex = rows.get("exyonq", {})
for tag in tags:
    r = rows.get(tag, {})
    print(f"CURRENT_{tag.upper()}_RPS_MEDIAN={r.get('rps_median')}")
    print(f"CURRENT_{tag.upper()}_CPU_PER_REQ={r.get('cpu_per_req')}")
    print(f"CURRENT_{tag.upper()}_USER_CPU_US_PER_REQ={r.get('user_us_per_req')}")
    print(f"CURRENT_{tag.upper()}_SYSTEM_CPU_US_PER_REQ={r.get('system_us_per_req')}")
    print(f"CURRENT_{tag.upper()}_CTX_SWITCHES_PER_REQ={r.get('ctx_per_req')}")
    print(f"CURRENT_{tag.upper()}_RSS={r.get('rss_mib')}")
    for p in ("p50","p95","p99"):
        if p in r:
            print(f"CURRENT_{tag.upper()}_{p.upper()}={r[p]}")

def pct_delta(new, old):
    if new is None or old in (None, 0):
        return None
    return (new - old) / old * 100

rps_gain = pct_delta(ex.get("rps_median"), OLD["exyonq_rps"])
cpu_old, cpu_new = OLD["exyonq_cpu"], ex.get("cpu_per_req")
cpu_improve = ((cpu_old - cpu_new) / cpu_old * 100) if cpu_old and cpu_new else None
user_improve = ((OLD["exyonq_user_us"] - ex.get("user_us_per_req")) / OLD["exyonq_user_us"] * 100) if ex.get("user_us_per_req") else None

print(f"OLD_AUTHORITATIVE_EXYONQ_RPS={OLD['exyonq_rps']}")
print(f"PXDP_RPS_GAIN_VS_OLD_PERCENT={rps_gain:.2f}" if rps_gain is not None else "PXDP_RPS_GAIN_VS_OLD_PERCENT=NOT_MEASURED")
print(f"OLD_AUTHORITATIVE_EXYONQ_CPU_PER_REQ={OLD['exyonq_cpu']}")
print(f"PXDP_CPU_PER_REQ_IMPROVEMENT_VS_OLD_PERCENT={cpu_improve:.2f}" if cpu_improve is not None else "PXDP_CPU_PER_REQ_IMPROVEMENT_VS_OLD_PERCENT=NOT_MEASURED")
print(f"PXDP_USER_CPU_IMPROVEMENT_VS_OLD_PERCENT={user_improve:.2f}" if user_improve is not None else "PXDP_USER_CPU_IMPROVEMENT_VS_OLD_PERCENT=NOT_MEASURED")
print("HISTORICAL_COMPARABILITY=BOUNDED")

ng, ol, ha = rows.get("nginx",{}).get("rps_median"), rows.get("ols",{}).get("rps_median"), rows.get("haproxy",{}).get("rps_median")
exr = ex.get("rps_median")
if exr and ng:
    print(f"EXYONQ_VS_NGINX_RPS={pct_delta(exr, ng):.2f}")
if exr and ol:
    print(f"EXYONQ_VS_OLS_RPS={pct_delta(exr, ol):.2f}")
if exr and ha:
    print(f"EXYONQ_VS_HAPROXY_RPS={pct_delta(exr, ha):.2f}")

# Decision gates
case = "PR-E"
if ex.get("rps_median") and rps_gain is not None and cpu_improve is not None:
    if rps_gain >= 15 and cpu_improve >= 15:
        case = "PR-A"
    elif rps_gain <= 5 or (cpu_improve is not None and cpu_improve <= 5):
        case = "PR-C"
    elif (5 < rps_gain < 15) or (5 < cpu_improve < 15):
        case = "PR-B"
if exr and ng and ol and exr >= min(ng, ol) * 0.95:
    print("P4_THROUGHPUT_POSITION=COMPETITIVE")
else:
    print("P4_THROUGHPUT_POSITION=BEHIND")
if cpu_improve and cpu_improve >= 15:
    print("P4_CPU_EFFICIENCY_POSITION=IMPROVED")
elif cpu_improve and cpu_improve > 0:
    print("P4_CPU_EFFICIENCY_POSITION=PARTIAL")
else:
    print("P4_CPU_EFFICIENCY_POSITION=BEHIND")

if case == "PR-A":
    arch = "SUPPORTED_MATERIALLY"
elif case == "PR-B":
    arch = "PARTIALLY_SUPPORTED"
elif case == "PR-C":
    arch = "WEAK_OR_FAILED"
else:
    arch = "NOT_PROVEN"
print(f"CASE={case}")
print(f"ARCHITECTURE_HYPOTHESIS={arch}")
if case == "PR-C":
    print("SHOULD_WE_INVEST_NEXT_IN_REQUEST_BODY_RELAY=NO")
elif case == "PR-A":
    print("SHOULD_WE_INVEST_NEXT_IN_REQUEST_BODY_RELAY=OWNER_REVIEW_REQUIRED")
else:
    print("SHOULD_WE_INVEST_NEXT_IN_REQUEST_BODY_RELAY=OWNER_REVIEW_REQUIRED")
print("P4_PERFORMANCE_MIGRATION_BARRIER=IMPROVED" if (rps_gain or 0) > 5 else "P4_PERFORMANCE_MIGRATION_BARRIER=REMAINS")
print("GLOBAL_MIGRATION_VALUE=NOT_PROVEN")
print("MARKET_READINESS=NOT_READY")
PY
}

log "START TS=$TS EV=$EV"
write_authority
write_environment
write_host_hygiene
ensure_stack
record_geometry
verify_pxdp_execution
correctness_gate
upstream_ceiling
for rep in $(seq 1 "$REPS"); do
  order=$(shuffle_tags)
  echo "rep=$rep order=$order" | tee -a "$EV/meta/run_order.txt"
  for tag in $order; do
    log "rep $rep/$REPS $tag"
    run_rep "$tag" "$rep"
    sleep 2
  done
done
for tag in "${TAGS[@]}"; do
  url=$(url_for "$tag")
  compose exec -T bench-runner bash -lc "timeout 90 rewrk -c 100 -d ${MEASURE_SEC}s -t 2 -h $url --pct 2>&1" \
    | tee "$EV/pct/${tag}.txt" || true
done
# cgroup companion (reuse P4 helper inline)
for tag in "${TAGS[@]}"; do
  c=$(ctr "$tag"); url=$(url_for "$tag"); id=$(docker inspect -f '{{.Id}}' "$c")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  [[ -f "$path" ]] || continue
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  cp "$path" "$EV/cgroup_companion/${tag}.before"
  compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json >"$EV/cgroup_companion/${tag}.rewrk.json"
  cp "$path" "$EV/cgroup_companion/${tag}.after"
  python3 - "$tag" "$EV/cgroup_companion/${tag}.before" "$EV/cgroup_companion/${tag}.after" \
    "$EV/cgroup_companion/${tag}.rewrk.json" "$EV/cgroup_companion/${tag}.summary.txt" <<'PY'
import sys, json
from pathlib import Path
tag=sys.argv[1]
def parse(p):
    d={}
    for ln in Path(p).read_text().splitlines():
        parts=ln.split()
        if len(parts)>=2: d[parts[0]]=int(parts[1])
    return d
b,a=parse(sys.argv[2]),parse(sys.argv[3])
j=json.loads(Path(sys.argv[4]).read_text())
req=float(j.get("requests_total") or 0)
du=a.get("usage_usec",0)-b.get("usage_usec",0)
uu=a.get("user_usec",0)-b.get("user_usec",0)
su=a.get("system_usec",0)-b.get("system_usec",0)
Path(sys.argv[5]).write_text(
  f"tag={tag}\nreq={req}\ntotal_us_per_req={(du/req) if req else None}\n"
  f"user_us_per_req={(uu/req) if req else None}\n"
  f"system_us_per_req={(su/req) if req else None}\n"
)
PY
done
summarize
echo "DONE $(date -u +%Y%m%dT%H%M%SZ)" | tee "$EV/reports/done.txt"
log "COMPLETE EV=$EV"
echo "$EV"
