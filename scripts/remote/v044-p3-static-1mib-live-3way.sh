#!/usr/bin/env bash
# V044_P3_STATIC_1MIB_COMPETITIVE_ADMISSION_BASELINE — ExyonQ vs NGINX vs OLS.
# PRODUCT_MUTATION=NO. P4=NO. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
#
# Contract sources (must agree):
#   benchmarks/scenarios/perf/scenarios.toml [p3]
#     path=/site/1m.bin connections=100 duration=30s
#   Competitive live-3way meter (same as P1 seal harness):
#     rewrk -c 100 -t 2; warmup 20s; measure 30s; 5 reps;
#     cpuset 0-7; CpuQuota=0; Docker internal; RAW WAF OFF.
set -euo pipefail

WS="${V044_P3_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${V044_P3_3WAY_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_P3_3WAY_EV:-$WS/.exyonq-local-evidence/v044-p3-static-1mib-3way-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
PATH_P3="/site/1m.bin"
WARMUP_SEC=20
MEASURE_SEC=30
REPS=5
EXPECTED_BYTES=1048576
EXPECTED_EXYONQ_SHA256="${EXPECTED_EXYONQ_SHA256:-09301dc867a4161ebf1da24635f97df14acffcc25f8211b1e475b97b6defe36f}"

mkdir -p "$EV"/{runs,stats,pct,meta,reports,sanity,geometry,cgroup_companion}
cd "$WS"
log() { echo "[p3-3way] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }
compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" --profile bench "$@"; }

ctr() {
  case "$1" in
    exyonq) echo "${PROJECT}-exyonq-1" ;;
    nginx) echo "${PROJECT}-nginx-stable-1" ;;
    ols) echo "${PROJECT}-openlitespeed-latest-1" ;;
  esac
}
url_for() {
  case "$1" in
    exyonq) echo "http://exyonq:8080${PATH_P3}" ;;
    nginx) echo "http://nginx-stable:8080${PATH_P3}" ;;
    ols) echo "http://openlitespeed-latest:8088${PATH_P3}" ;;
  esac
}

prepare() {
  {
    echo "P3_CONTRACT_SOURCE=benchmarks/scenarios/perf/scenarios.toml [p3] + P1 live-3way competitive meter"
    echo "P3_PAYLOAD_BYTES=$EXPECTED_BYTES"
    echo "P3_PROTOCOL=HTTP/1.1"
    echo "P3_KEEPALIVE=rewrk default keepalive (connection reuse)"
    echo "P3_CONNECTIONS=100"
    echo "P3_LOAD_THREADS=2"
    echo "P3_WARMUP_SECONDS=$WARMUP_SEC"
    echo "P3_MEASURE_SECONDS=$MEASURE_SEC"
    echo "P3_REPETITIONS=$REPS"
    echo "P3_CPUSET=0-7"
    echo "P3_CPU_QUOTA=0"
    echo "P3_WAF_MODE=explicit enabled=false mode=disabled (RAW)"
    echo "P3_EXECUTION_MODEL=Docker compose project=$PROJECT overlay=p1-authoritative"
    echo "P3_PATH=$PATH_P3"
    echo "PRODUCT_AUTHORITY_NOTE=binary must match EXPECTED_EXYONQ_SHA256 (product commit 06742e1b)"
  } | tee "$EV/meta/p3_contract.txt"

  uptime | tee "$EV/meta/host_uptime_before.txt"
  df -h / /tmp 2>/dev/null | tee "$EV/meta/disk_pressure.txt" || true
  docker ps --format '{{.Names}}' | tee "$EV/meta/host_containers.txt" || true

  for tag in exyonq nginx ols; do
    c=$(ctr "$tag")
    docker update --cpuset-cpus "0-7" "$c" >/dev/null
    docker inspect -f 'Cpuset={{.HostConfig.CpusetCpus}} CpuQuota={{.HostConfig.CpuQuota}}' "$c" \
      | tee -a "$EV/meta/cpuset.txt"
  done

  EXY_SHA=$(docker exec "$(ctr exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "EXYONQ_BINARY_SHA256=$EXY_SHA" | tee "$EV/meta/exyonq_sha.txt"
  if [[ -n "$EXPECTED_EXYONQ_SHA256" && "$EXY_SHA" != "$EXPECTED_EXYONQ_SHA256" ]]; then
    log "FAIL: ExyonQ SHA $EXY_SHA != expected $EXPECTED_EXYONQ_SHA256"
    exit 2
  fi

  # Identity / CAUSE3C observation (read-only)
  {
    echo "=== exyonq id ==="
    docker exec "$(ctr exyonq)" id || true
    echo "=== nginx id / workers ==="
    docker exec "$(ctr nginx)" id || true
    docker exec "$(ctr nginx)" sh -c 'ps aux 2>/dev/null | head -20' || true
    echo "=== ols id / workers ==="
    docker exec "$(ctr ols)" id || true
    docker exec "$(ctr ols)" sh -c 'ps aux 2>/dev/null | head -20' || true
    echo "=== pipe-max-size (exyonq ns) ==="
    docker exec "$(ctr exyonq)" sh -c 'cat /proc/sys/fs/pipe-max-size 2>/dev/null' || true
    echo "=== waf stanza ==="
    docker exec "$(ctr exyonq)" sh -c 'grep -A5 "^\[waf\]" /bench/bench.toml 2>/dev/null' || true
    echo "=== workers env ==="
    docker exec "$(ctr exyonq)" sh -c 'env | grep -E "WORKER|EPOLL|WAF" | sort' || true
  } | tee "$EV/geometry/identity.txt"

  for tag in exyonq nginx ols; do
    u=$(url_for "$tag")
    out=$(compose exec -T bench-runner curl -sS -m 10 -o "/tmp/${tag}-1m.bin" -w "%{http_code} %{size_download}" "$u")
    echo "${tag}_probe=$out" | tee -a "$EV/meta/http_probe.txt"
    code=${out%% *}
    size=${out##* }
    if [[ "$code" != "200" || "$size" != "$EXPECTED_BYTES" ]]; then
      log "FAIL probe $tag code=$code size=$size expected=$EXPECTED_BYTES"
      exit 2
    fi
  done

  # Body hash equality across trio
  compose exec -T bench-runner sha256sum /tmp/exyonq-1m.bin /tmp/nginx-1m.bin /tmp/ols-1m.bin \
    | tee "$EV/sanity/trio_1m.sha"
  H_EX=$(compose exec -T bench-runner sha256sum /tmp/exyonq-1m.bin | awk '{print $1}')
  H_NG=$(compose exec -T bench-runner sha256sum /tmp/nginx-1m.bin | awk '{print $1}')
  H_OL=$(compose exec -T bench-runner sha256sum /tmp/ols-1m.bin | awk '{print $1}')
  if [[ "$H_EX" != "$H_NG" || "$H_EX" != "$H_OL" ]]; then
    log "FAIL hash mismatch ex=$H_EX ng=$H_NG ol=$H_OL"
    exit 2
  fi
  echo "GET_1MIB_HASH_MATCH=YES hash=$H_EX" | tee "$EV/sanity/hash_match.txt"

  # HEAD + keepalive sanity (ExyonQ required; rivals best-effort)
  {
    for tag in exyonq nginx ols; do
      u=$(url_for "$tag")
      head_out=$(compose exec -T bench-runner curl -sS -m 10 -I -o /tmp/${tag}-head.hdr -w "%{http_code} %{size_download}" "$u" || echo "FAIL 0")
      echo "${tag}_HEAD=$head_out"
      ka=$(compose exec -T bench-runner bash -lc "
        curl -sS -m 10 --http1.1 \"$u\" -o /tmp/${tag}-ka1.bin -w '%{http_code} %{size_download} '
        curl -sS -m 10 --http1.1 \"$u\" -o /tmp/${tag}-ka2.bin -w '%{http_code} %{size_download}'
      " || echo "FAIL")
      echo "${tag}_KEEPALIVE_SERIAL=$ka"
    done
  } | tee "$EV/sanity/head_keepalive.txt"
}

cgroup_companion() {
  local tag=$1
  local c url id path
  c=$(ctr "$tag")
  url=$(url_for "$tag")
  id=$(docker inspect -f '{{.Id}}' "$c")
  path="/sys/fs/cgroup/system.slice/docker-${id}.scope/cpu.stat"
  if [[ ! -f "$path" ]]; then
    echo "NO_CGROUP_CPU_STAT path=$path" | tee "$EV/cgroup_companion/${tag}.missing"
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
out={
  "tag": tag,
  "req": req,
  "total_us_per_req": (du/req) if req else None,
  "user_us_per_req": (uu/req) if req else None,
  "system_us_per_req": (su/req) if req else None,
}
Path(sys.argv[5]).write_text(
  f"req={req}\ntotal_us_per_req={out['total_us_per_req']}\n"
  f"user_us_per_req={out['user_us_per_req']}\nsystem_us_per_req={out['system_us_per_req']}\n"
)
Path(sys.argv[5].replace('.summary.txt','.json')).write_text(json.dumps(out, indent=2)+"\n")
print(out)
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
  sleep 0.4
  compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
    >"$EV/runs/${tag}-rep${rep}.json"
  wait "$sp" 2>/dev/null || true
}

pct_probe() {
  local tag=$1
  local url
  url=$(url_for "$tag")
  compose exec -T bench-runner bash -lc \
    "rewrk -c 100 -d ${MEASURE_SEC}s -t 2 -h $url --pct 2>&1" \
    | tee "$EV/pct/${tag}.txt"
}

log "START TS=$TS EV=$EV PATH=$PATH_P3 PROJECT=$PROJECT"
log "restart trio for cleaner memory comparability"
for tag in exyonq nginx ols; do
  docker restart "$(ctr "$tag")" >/dev/null
done
sleep 10
prepare
for rep in $(seq 1 "$REPS"); do
  log "rep $rep/$REPS exyonq"
  run_rep exyonq "$rep"
  log "rep $rep/$REPS nginx"
  run_rep nginx "$rep"
  log "rep $rep/$REPS ols"
  run_rep ols "$rep"
done
log "percentile probes"
for tag in exyonq nginx ols; do
  pct_probe "$tag" || true
done
log "cgroup companion"
for tag in exyonq nginx ols; do
  cgroup_companion "$tag" || true
done

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
    for pct in ("50", "90", "95", "99"):
        m = re.search(rf"\|\s*{pct}%\s*\|\s*([\d.]+)ms", text)
        if m:
            out[f"p{pct}"] = float(m.group(1))
    m = re.search(r"(\d+)\s+Errors:\s*connection closed", text)
    out["probe_artifact_connection_closed"] = int(m.group(1)) if m else 0
    out["server_error_count"] = "NOT_MEASURED"
    return out

rows = {}
for tag in tags:
    rps_list, xfer_list, cpu_avgs, cpu_p95s, cpu_maxs, rss_avgs, rss_peaks = [], [], [], [], [], [], []
    for rep in range(1, 6):
        j = json.loads((ev / "runs" / f"{tag}-rep{rep}.json").read_text())
        rps = float(j.get("requests_avg") or 0)
        rps_list.append(rps)
        # transfer_rate is bytes/sec in rewrk json
        tr = j.get("transfer_rate")
        if tr is not None:
            xfer_list.append(float(tr) / (1024.0 * 1024.0))
        cpus, rss = parse_cpu_mem(ev / "stats" / f"{tag}-rep{rep}.cpu.txt")
        if cpus:
            cpu_avgs.append(statistics.mean(cpus))
            cpu_p95s.append(sorted(cpus)[max(0, int(math.ceil(0.95 * len(cpus)) - 1))])
            cpu_maxs.append(max(cpus))
        if rss:
            rss_avgs.append(statistics.mean(rss))
            rss_peaks.append(max(rss))
    cpu = median(cpu_avgs)
    rps = median(rps_list)
    cores = (cpu / 100.0) if cpu else None
    rpc = (rps / cores) if cores and cores > 0 else None
    cms = (1000.0 / rpc) if rpc else None
    pct = parse_pct(ev / "pct" / f"{tag}.txt")
    rows[tag] = {
        "rps_runs": rps_list,
        "rps_median": rps,
        "rps_cv": cv(rps_list),
        "throughput_mib_s_median": median(xfer_list),
        "cpu_avg": cpu,
        "cpu_p95": median(cpu_p95s),
        "cpu_max": median(cpu_maxs),
        "req_per_core": rpc,
        "core_ms_per_req": cms,
        "rss_avg_mib": median(rss_avgs),
        "rss_peak_mib": median(rss_peaks),
        **pct,
    }
    print(f"=== {tag} ===")
    for k, v in rows[tag].items():
        print(f"{k}={v}")

ex, ng, ol = rows["exyonq"], rows["nginx"], rows["ols"]

def pct_delta(a, b):
    return None if a is None or b is None or b == 0 else 100.0 * (a - b) / b

vs_ng = pct_delta(ex["core_ms_per_req"], ng["core_ms_per_req"])
vs_ol = pct_delta(ex["core_ms_per_req"], ol["core_ms_per_req"])
rps_ng = pct_delta(ex["rps_median"], ng["rps_median"])
rps_ol = pct_delta(ex["rps_median"], ol["rps_median"])
print(f"EXYONQ_CPU_COST_VS_NGINX_PERCENT={vs_ng}")
print(f"EXYONQ_CPU_COST_VS_OLS_PERCENT={vs_ol}")
print(f"EXYONQ_RPS_VS_NGINX_PERCENT={rps_ng}")
print(f"EXYONQ_RPS_VS_OLS_PERCENT={rps_ol}")
print(f"EV={ev}")

summary = {
    "scenario": "P3",
    "path": "/site/1m.bin",
    "payload_bytes": 1048576,
    "rows": rows,
    "vs_nginx_pct": vs_ng,
    "vs_ols_pct": vs_ol,
    "rps_vs_nginx_pct": rps_ng,
    "rps_vs_ols_pct": rps_ol,
}
(ev / "reports" / "threeway_summary.json").write_text(json.dumps(summary, indent=2) + "\n")
PY

log "DONE EV=$EV"
