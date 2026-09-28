#!/usr/bin/env bash
# V044_P4_PROXY_1K_COMPETITIVE_ADMISSION_BASELINE — ExyonQ vs NGINX vs OLS.
# PRODUCT_MUTATION=NO. PERFORMANCE_OPTIMIZATION=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
#
# Contract (reconciled — no material disagreement):
#   benchmarks/scenarios/perf/scenarios.toml [p4]
#     label=Proxy 1 KiB path=/api/ connections=100 duration=30s
#   xtask/fixtures/scenarios.toml [p4] — identical
#   Upstream peer: tools/upstream exyonq-upstream → GET /api/ = 1024 B (1k.bin)
#   Proxy configs: benchmarks/configs/{exyonq/bench.toml,nginx/nginx.conf,openlitespeed/...}
# Competitive meter (P1/P3 live-3way authority):
#   rewrk -c 100 -t 2; warmup 20s; measure 30s; 5 reps;
#   cpuset 0-7; CpuQuota=0; Docker internal; RAW WAF OFF (bench.toml explicit).
set -euo pipefail

WS="${V044_P4_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${V044_P4_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_P4_EV:-$WS/.exyonq-local-evidence/v044-p4-proxy-1k-3way-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
IMAGE="${P4_EXYONQ_IMAGE:-v044p1-geom8-exyonq}"
PRODUCT_COMMIT="${V044_PRODUCT_COMMIT:-ade997a362e60d1dc6eafc984da454930600845a}"
EXPECTED_EXYONQ_SHA256="${EXPECTED_EXYONQ_SHA256:-e28c0abe18587f2d911d492be64cbec42372383e011417a6c578be52da5ba459}"
PATH_P4="/api/"
UPSTREAM_PATH="/api/"
WARMUP_SEC=20
MEASURE_SEC=30
REPS=5
EXPECTED_BYTES=1024

mkdir -p "$EV"/{runs,stats,pct,meta,reports,sanity,geometry,cgroup_companion,upstream}
cd "$WS"
log() { echo "[p4-3way] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }
compose() {
  local extra=()
  if [[ -f "$EV/meta/compose-geom8.yml" ]]; then
    extra=(-f "$EV/meta/compose-geom8.yml")
  fi
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@"
}

ctr() {
  case "$1" in
    exyonq) echo "${PROJECT}-exyonq-1" ;;
    nginx) echo "${PROJECT}-nginx-stable-1" ;;
    ols) echo "${PROJECT}-openlitespeed-latest-1" ;;
    upstream) echo "${PROJECT}-upstream-1" ;;
  esac
}

url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P4}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P4}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P4}" ;;
    upstream) echo "http://upstream:9000${UPSTREAM_PATH}" ;;
  esac
}

write_contract() {
  {
    echo "P4_CONTRACT_SOURCE=benchmarks/scenarios/perf/scenarios.toml [p4] + xtask/fixtures/scenarios.toml [p4] + bench docker configs"
    echo "P4_CONTRACT_RECONCILIATION_REQUIRED=NO"
    echo "P4_REQUEST_PATH=$PATH_P4"
    echo "P4_UPSTREAM_TYPE=exyonq-upstream (real HTTP/1.1 TCP peer)"
    echo "P4_UPSTREAM_PAYLOAD_BYTES=$EXPECTED_BYTES"
    echo "P4_PROTOCOL_CLIENT_SIDE=HTTP/1.1"
    echo "P4_PROTOCOL_UPSTREAM_SIDE=HTTP/1.1"
    echo "P4_KEEPALIVE_CLIENT=rewrk default (connection reuse)"
    echo "P4_KEEPALIVE_UPSTREAM=enabled (upstream Connection: keep-alive; nginx/ols persistConn)"
    echo "P4_CONNECTIONS=100"
    echo "P4_LOAD_THREADS=2"
    echo "P4_WARMUP_SECONDS=$WARMUP_SEC"
    echo "P4_MEASURE_SECONDS=$MEASURE_SEC"
    echo "P4_REPETITIONS=$REPS"
    echo "P4_CPUSET=0-7"
    echo "P4_CPU_QUOTA=0"
    echo "P4_EXECUTION_MODEL=Docker compose project=$PROJECT overlay=p1-authoritative"
    echo "P4_BENCHMARK_CATEGORY=RAW_TRANSPORT"
    echo "P4_BENCHMARK_CATEGORY_NOTE=ExyonQ bench.toml explicit WAF off + access/otel off; nginx access_log off; comparable minimal proxy path"
    echo "PRODUCT_COMMIT=$PRODUCT_COMMIT"
    echo "EXPECTED_EXYONQ_SHA256=$EXPECTED_EXYONQ_SHA256"
  } | tee "$EV/meta/p4_contract.txt"
}

ensure_stack() {
  unset EXYONQ_WORKER_THREADS EXYONQ_ACCEPT_WORKERS EXYONQ_EPOLL_POOL_THREADS || true
  export P1_EXYONQ_IMAGE="$IMAGE"
  if [[ ! -f "$EV/meta/compose-geom8.yml" ]]; then
    cat >"$EV/meta/compose-geom8.yml" <<EOF
services:
  exyonq:
    image: ${IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
EOF
  fi
  compose up -d --no-deps exyonq nginx-stable openlitespeed-latest upstream
  compose --profile bench up -d --no-deps bench-runner 2>/dev/null || true
  sleep 8
  for tag in exyonq nginx ols upstream; do
    docker update --cpuset-cpus "0-7" "$(ctr "$tag")" >/dev/null 2>&1 || true
  done
}

record_geometry() {
  {
    echo "=== startup ==="
    docker logs "$(ctr exyonq)" 2>&1 | egrep -i "listening|keepalive pool geometry|workers" | tail -5 || true
    echo "=== env ==="
    docker exec "$(ctr exyonq)" sh -c 'env | egrep "WORKER|ACCEPT|EPOLL|WAF" | sort; nproc' || true
    echo "=== waf ==="
    docker exec "$(ctr exyonq)" sh -c 'grep -A5 "^\[waf\]" /bench/bench.toml 2>/dev/null' || true
    echo "=== logging ==="
    docker exec "$(ctr exyonq)" sh -c 'grep -A3 "^\[logging" /bench/bench.toml 2>/dev/null' || true
  } | tee "$EV/geometry/exyonq.txt"
  EXY_SHA=$(docker exec "$(ctr exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "EXYONQ_BINARY_SHA256=$EXY_SHA" | tee "$EV/meta/exyonq_sha.txt"
  if [[ -n "$EXPECTED_EXYONQ_SHA256" && "$EXY_SHA" != "$EXPECTED_EXYONQ_SHA256" ]]; then
    log "FAIL: ExyonQ SHA mismatch got=$EXY_SHA expected=$EXPECTED_EXYONQ_SHA256"
    exit 2
  fi
  UP_SHA=$(docker exec "$(ctr upstream)" sha256sum /usr/local/bin/exyonq-upstream | awk '{print $1}')
  echo "UPSTREAM_BINARY_SHA256=$UP_SHA" | tee "$EV/meta/upstream_sha.txt"
  docker exec "$(ctr upstream)" sh -c 'ss -lntp 2>/dev/null || netstat -lntp 2>/dev/null' \
    | tee "$EV/meta/upstream_listen.txt" || true
}

correctness_gate() {
  local fail=0
  # Upstream direct body
  local udir=$(url_for upstream)
  local uout
  uout=$(compose exec -T bench-runner curl -sS -m 10 -o /tmp/upstream-1k.bin -w "%{http_code} %{size_download}" "$udir")
  echo "upstream_direct=$uout" | tee "$EV/sanity/upstream_direct.txt"
  local ucode=${uout%% *} usize=${uout##* }
  if [[ "$ucode" != "200" || "$usize" != "$EXPECTED_BYTES" ]]; then
    log "FAIL upstream direct code=$ucode size=$usize"
    fail=1
  fi
  compose exec -T bench-runner sha256sum /tmp/upstream-1k.bin | tee "$EV/sanity/upstream_body.sha"
  local uh
  uh=$(compose exec -T bench-runner sha256sum /tmp/upstream-1k.bin | awk '{print $1}')

  for tag in exyonq nginx ols; do
    local u out code size
    u=$(url_for "$tag")
    out=$(compose exec -T bench-runner curl -sS -m 10 -o "/tmp/${tag}-api-1k.bin" -w "%{http_code} %{size_download}" "$u")
    echo "${tag}_proxy=$out" | tee -a "$EV/sanity/proxy_probe.txt"
    code=${out%% *}; size=${out##* }
    if [[ "$code" != "200" || "$size" != "$EXPECTED_BYTES" ]]; then
      log "FAIL $tag proxy code=$code size=$size"
      fail=1
    fi
    local h
    h=$(compose exec -T bench-runner sha256sum "/tmp/${tag}-api-1k.bin" | awk '{print $1}')
    echo "${tag}_hash=$h" | tee -a "$EV/sanity/proxy_hash.txt"
    if [[ "$h" != "$uh" ]]; then
      log "FAIL $tag body hash mismatch"
      fail=1
    fi
    # serial keepalive on same connection
    compose exec -T bench-runner bash -lc "
      curl -sS -m 10 --http1.1 '$u' -o /tmp/${tag}-ka1.bin -w '%{http_code} %{size_download} '
      curl -sS -m 10 --http1.1 '$u' -o /tmp/${tag}-ka2.bin -w '%{http_code} %{size_download}'
    " | tee -a "$EV/sanity/keepalive_serial.txt"
  done
  if [[ "$fail" -ne 0 ]]; then
    echo "P4_STATUS=BLOCKED_PRODUCT_DEFECT" | tee "$EV/reports/status.txt"
    exit 2
  fi
  echo "CORRECTNESS_GATE=PASS BODY_HASH_MATCH=YES" | tee "$EV/sanity/correctness.txt"
}

# Fail fast if a rival proxy cannot sustain load (e.g. nginx without upstream keepalive).
proxy_comparability_smoke() {
  local tag url out rps_ex rps_ng rps_ol min_peer
  log "proxy comparability smoke (single 30s rep each)"
  for tag in exyonq nginx ols; do
    url=$(url_for "$tag")
    compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
    compose exec -T bench-runner timeout $((MEASURE_SEC + 20)) rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
      >"$EV/sanity/smoke-${tag}.json"
    python3 -c "import json; print(json.load(open('$EV/sanity/smoke-${tag}.json')).get('requests_avg'))" \
      | tee "$EV/sanity/smoke-${tag}.rps"
  done
  rps_ex=$(cat "$EV/sanity/smoke-exyonq.rps")
  rps_ng=$(cat "$EV/sanity/smoke-nginx.rps")
  rps_ol=$(cat "$EV/sanity/smoke-ols.rps")
  min_peer=$(python3 -c "print(min(float('$rps_ex'), float('$rps_ol')))")
  python3 - "$rps_ex" "$rps_ng" "$rps_ol" "$min_peer" <<'PY' | tee "$EV/sanity/proxy_comparability_smoke.txt"
import sys
ex, ng, ol, min_peer = map(float, sys.argv[1:5])
print(f"SMOKE_EXYONQ_RPS={ex}")
print(f"SMOKE_NGINX_RPS={ng}")
print(f"SMOKE_OLS_RPS={ol}")
print(f"SMOKE_MIN_PEER_RPS={min_peer}")
ratio = ng / min_peer if min_peer else 0
print(f"SMOKE_NGINX_TO_MIN_PEER_RATIO={ratio:.4f}")
if ratio < 0.25:
    print("P4_COMPARABILITY=FAIL")
    print("NGINX_RPS_STATUS=INVALID_RUN_HARNESS_DEFECT_SUSPECTED")
    print("HARNESS_DEFECT=nginx_upstream_connect_under_load")
    sys.exit(2)
print("P4_COMPARABILITY_SMOKE=PASS")
PY
}

upstream_ceiling() {
  local url statsjson
  url=$(url_for upstream)
  statsjson="$EV/upstream/direct.cpu.txt"
  : >"$statsjson"
  timeout $((MEASURE_SEC + WARMUP_SEC + 25)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$(ctr upstream)" \
    >"$statsjson" 2>/dev/null &
  local sp=$!
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  compose exec -T bench-runner timeout $((MEASURE_SEC + 20)) rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
    >"$EV/upstream/direct.json"
  compose exec -T bench-runner timeout 90 rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --pct 2>&1 \
    | tee "$EV/upstream/direct.pct.txt" || true
  wait "$sp" 2>/dev/null || true
  python3 - "$EV/upstream" <<'PY' | tee "$EV/upstream/direct.summary.txt"
import json,re,statistics,sys
from pathlib import Path
d=Path(sys.argv[1])
j=json.loads((d/"direct.json").read_text())
rps=float(j.get("requests_avg") or 0)
ptxt=(d/"direct.pct.txt").read_text(errors="replace")
pct={}
for p in ("50","95","99"):
    m=re.search(rf"\|\s*{p}%\s*\|\s*([\d.]+)ms", ptxt)
    if m: pct[p]=float(m.group(1))
out={"rps":rps,**{f"p{k}":v for k,v in pct.items()}}
(d/"direct.summary.json").write_text(json.dumps(out,indent=2)+"\n")
print(f"UPSTREAM_DIRECT_RPS={rps}")
for k,v in pct.items(): print(f"UPSTREAM_DIRECT_{k.upper()}={v}")
PY
}

run_rep() {
  local tag=$1 rep=$2
  local c url statsjson
  c=$(ctr "$tag")
  url=$(url_for "$tag")
  statsjson="$EV/stats/${tag}-rep${rep}.cpu.txt"
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  : >"$statsjson"
  timeout $((MEASURE_SEC + 15)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$statsjson" 2>/dev/null &
  local sp=$!
  sleep 0.3
  compose exec -T bench-runner timeout $((MEASURE_SEC + 20)) rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
    >"$EV/runs/${tag}-rep${rep}.json"
  wait "$sp" 2>/dev/null || true
}

cgroup_companion() {
  local tag=$1 c url id path
  c=$(ctr "$tag")
  url=$(url_for "$tag")
  id=$(docker inspect -f '{{.Id}}' "$c")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  if [[ ! -f "$path" ]]; then
    echo "NO_CGROUP_CPU_STAT" | tee "$EV/cgroup_companion/${tag}.missing"
    return 0
  fi
  compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
  cp "$path" "$EV/cgroup_companion/${tag}.before"
  compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
    >"$EV/cgroup_companion/${tag}.rewrk.json"
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
print(f"{tag}_CGROUP_TOTAL_US_PER_REQ={(du/req) if req else 'NOT_MEASURED'}")
PY
}

summarize() {
  python3 - "$EV" <<'PY' | tee "$EV/reports/threeway_terminal.txt"
import json, re, statistics, math, sys
from pathlib import Path

ev = Path(sys.argv[1])
ansi = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
tags = ("exyonq", "nginx", "ols")

def median(xs):
    return statistics.median(xs) if xs else None

def cv(xs):
    if not xs or len(xs) < 2:
        return None
    m = statistics.mean(xs)
    return (statistics.stdev(xs) / m) if m else None

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
        unit = m2.group(3)
        mul = {"KiB": 1 / 1024, "MiB": 1, "GiB": 1024}[unit]
        rss.append(val * mul)
    return cpus, rss

def parse_pct(path):
    text = path.read_text(errors="replace") if path.exists() else ""
    out = {}
    for pct in ("50", "95", "99"):
        m = re.search(rf"\|\s*{pct}%\s*\|\s*([\d.]+)ms", text)
        if m:
            out[f"p{pct}"] = float(m.group(1))
    m = re.search(r"(\d+)\s+Errors:\s*connection closed", text)
    out["probe_artifact_connection_closed"] = int(m.group(1)) if m else 0
    return out

rows = {}
for tag in tags:
    rps_list, cpu_avgs, rss_avgs, rss_peaks = [], [], [], []
    for rep in range(1, 6):
        j = json.loads((ev / "runs" / f"{tag}-rep{rep}.json").read_text())
        rps_list.append(float(j.get("requests_avg") or 0))
        cpus, rss = parse_cpu_mem(ev / "stats" / f"{tag}-rep{rep}.cpu.txt")
        if cpus:
            cpu_avgs.append(statistics.mean(cpus))
        if rss:
            rss_avgs.append(statistics.mean(rss))
            rss_peaks.append(max(rss))
    cpu = median(cpu_avgs)
    rps = median(rps_list)
    core_ms = (cpu / 100 * 8 * 1000 / rps) if cpu and rps else None
    pct = parse_pct(ev / "pct" / f"{tag}.txt")
    cg = ev / "cgroup_companion" / f"{tag}.summary.txt"
    cgroup = {}
    if cg.exists():
        for ln in cg.read_text().splitlines():
            if "=" in ln:
                k, v = ln.split("=", 1)
                cgroup[k] = v
    rows[tag] = {
        "rps_runs": rps_list,
        "rps_median": rps,
        "rps_cv": cv(rps_list),
        "cpu_pct": cpu,
        "core_ms_per_req": core_ms,
        "rss_avg_mib": median(rss_avgs),
        "rss_peak_mib": median(rss_peaks),
        "cgroup_total_us_per_req": float(cgroup["total_us_per_req"]) if cgroup.get("total_us_per_req") not in (None, "None") else None,
        "cgroup_user_us_per_req": float(cgroup["user_us_per_req"]) if cgroup.get("user_us_per_req") not in (None, "None") else None,
        "cgroup_system_us_per_req": float(cgroup["system_us_per_req"]) if cgroup.get("system_us_per_req") not in (None, "None") else None,
        **pct,
    }

def winner(key, lower=False):
    vals = {t: rows[t].get(key) for t in tags if rows[t].get(key) is not None}
    if not vals:
        return "NOT_MEASURED"
    return min(vals, key=vals.get) if lower else max(vals, key=vals.get)

(ev / "summary.json").write_text(json.dumps(rows, indent=2) + "\n")

for tag in tags:
    r = rows[tag]
    print(f"P4_{tag.upper()}_RPS_MEDIAN={r['rps_median']}")
    print(f"P4_{tag.upper()}_RPS_REPETITIONS={r['rps_runs']}")
    print(f"P4_{tag.upper()}_RPS_CV={r['rps_cv']}")
    print(f"P4_{tag.upper()}_CPU_PER_REQ={r['core_ms_per_req']}")
    print(f"P4_{tag.upper()}_RSS_AVG={r['rss_avg_mib']}")
    print(f"P4_{tag.upper()}_RSS_PEAK={r['rss_peak_mib']}")
    for p in ("p50", "p95", "p99"):
        if p in r:
            print(f"P4_{tag.upper()}_{p.upper()}={r[p]}")
    if r.get("cgroup_total_us_per_req") is not None:
        print(f"P4_{tag.upper()}_USER_CPU_US_PER_REQ={r.get('cgroup_user_us_per_req')}")
        print(f"P4_{tag.upper()}_SYSTEM_CPU_US_PER_REQ={r.get('cgroup_system_us_per_req')}")

print(f"P4_WINNER_RPS={winner('rps_median')}")
print(f"P4_WINNER_CPU={winner('core_ms_per_req', True)}")
print(f"P4_WINNER_MEMORY={winner('rss_avg_mib', True)}")
print(f"P4_WINNER_P50={winner('p50', True)}")
print(f"P4_WINNER_P95={winner('p95', True)}")
print(f"P4_WINNER_P99={winner('p99', True)}")
print(f"P4_WINNER_STABILITY={winner('rps_cv', True)}")

ex, ng, ol = rows["exyonq"]["rps_median"], rows["nginx"]["rps_median"], rows["ols"]["rps_median"]
if ex and ng:
    print(f"EXYONQ_VS_NGINX_RPS_PERCENT={(ex-ng)/ng*100:.2f}")
if ex and ol:
    print(f"EXYONQ_VS_OLS_RPS_PERCENT={(ex-ol)/ol*100:.2f}")

# upstream validity
us = json.loads((ev / "upstream" / "direct.summary.json").read_text()) if (ev / "upstream" / "direct.summary.json").exists() else {}
up_rps = us.get("rps")
if up_rps and ex:
    headroom = up_rps / ex if ex else None
    print(f"UPSTREAM_DIRECT_RPS={up_rps}")
    print(f"UPSTREAM_TO_EXYONQ_HEADROOM_RATIO={headroom:.2f}" if headroom else "UPSTREAM_TO_EXYONQ_HEADROOM_RATIO=NOT_MEASURED")
    if headroom and headroom < 1.15:
        print("P4_VALIDITY=REVIEW_UPSTREAM_HEADROOM_LOW")
    else:
        print("P4_VALIDITY=PASS")
PY
}

log "START TS=$TS EV=$EV PATH=$PATH_P4"
write_contract
ensure_stack
record_geometry
correctness_gate
log "upstream ceiling"
upstream_ceiling
log "restart trio for memory comparability"
for tag in exyonq nginx ols; do docker restart "$(ctr "$tag")" >/dev/null; done
sleep 12
record_geometry
proxy_comparability_smoke
for rep in $(seq 1 "$REPS"); do
  for tag in exyonq nginx ols; do
    log "rep $rep/$REPS $tag"
    run_rep "$tag" "$rep"
    python3 -c "import json; print(json.load(open('$EV/runs/${tag}-rep${rep}.json')).get('requests_avg'))" \
      >>"$EV/stats/${tag}_rps.txt"
    sleep 2
  done
done
log "percentile probes"
for tag in exyonq nginx ols; do
  url=$(url_for "$tag")
  compose exec -T bench-runner bash -lc "timeout 90 rewrk -c 100 -d ${MEASURE_SEC}s -t 2 -h $url --pct 2>&1" \
    | tee "$EV/pct/${tag}.txt" || true
done
log "cgroup companion"
for tag in exyonq nginx ols; do cgroup_companion "$tag" || true; done
summarize
echo "DONE $(date -u +%Y%m%dT%H%M%SZ)" | tee "$EV/reports/done.txt"
log "COMPLETE EV=$EV"
echo "$EV"
