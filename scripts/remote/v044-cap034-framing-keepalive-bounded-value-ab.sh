#!/usr/bin/env bash
# V044_CAP034_FRAMING_KEEPALIVE_BOUNDED_VALUE_AB — A0 vs A1 matched alternating runs.
# EXPERIMENT_PRODUCT_MUTATION=YES_LIMITED_TO_A1. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
set -euo pipefail

WS="${CAP034_AB_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${CAP034_AB_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${CAP034_AB_EV:-$WS/.exyonq-local-evidence/v044-cap034-framing-keepalive-ab-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
IMAGE="${CAP034_AB_IMAGE:-v044p1-geom8-exyonq:latest}"
EXY_BIN="${CAP034_AB_BINARY:-$WS/.exyonq-local-evidence/cap034-ab-exyonq}"
PRODUCT_COMMIT="${EXPERIMENT_COMMIT:-$(git -C "$WS" rev-parse HEAD 2>/dev/null || echo NOT_MEASURED)}"
PATH_P4="/api/"
WARMUP_SEC=20
MEASURE_SEC=30
REPS="${CAP034_AB_REPS:-5}"
CONCURRENCY=100
THREADS=2
EXPECTED_BYTES=1024

mkdir -p "$EV"/{runs,stats,meta,reports,sanity,wire,perf-stat,connection}
cd "$WS"
log() { echo "[cap034-ab] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }

compose() {
  local extra=()
  if [[ -f "$EV/meta/compose-geom8.yml" ]]; then
    extra+=(-f "$EV/meta/compose-geom8.yml")
  fi
  if [[ -f "$EV/meta/compose-arm.yml" ]]; then
    extra+=(-f "$EV/meta/compose-arm.yml")
  fi
  docker compose -f "$FULL_COMPOSE" -f "$OVER" "${extra[@]}" -p "$PROJECT" --profile bench "$@"
}

ctr_exyonq() { echo "${PROJECT}-exyonq-1"; }
ctr_upstream() { echo "${PROJECT}-upstream-1"; }

url_exyonq() { echo "http://exyonq:8080${PATH_P4}"; }

write_meta() {
  {
    echo "WIP=V044_CAP034_FRAMING_KEEPALIVE_BOUNDED_VALUE_AB"
    echo "NON_PRODUCTION_EXPERIMENT=YES"
    echo "RUN_ID=v044-cap034-framing-keepalive-ab-$TS"
    echo "EXPERIMENT_COMMIT=$PRODUCT_COMMIT"
    echo "EXYONQ_IMAGE=$IMAGE"
    echo "P4_CONTRACT_UNCHANGED=YES"
    echo "HAPROXY_IN_THIS_AB=NOT_MEASURED"
    echo "REPETITIONS_PER_ARM=$REPS"
    echo "MATCHED_ALTERNATING=A0,A1,..."
    echo "A0_FRAMING=cap034_chunked_close"
    echo "A1_FRAMING=safe_known_length_keepalive"
    uname -a
    nproc
  } | tee "$EV/meta/run_meta.txt"
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
  compose up -d --no-deps upstream exyonq 2>/dev/null || true
  compose --profile bench up -d --no-deps bench-runner 2>/dev/null || true
  sleep 4
  docker update --cpuset-cpus "0-7" "$(ctr_exyonq)" >/dev/null 2>&1 || true
  docker update --cpuset-cpus "0-7" "$(ctr_upstream)" >/dev/null 2>&1 || true
  if [[ -f "$EXY_BIN" ]]; then
    docker cp "$EXY_BIN" "$(ctr_exyonq):/usr/local/bin/exyonq"
    docker restart "$(ctr_exyonq)" >/dev/null
    sleep 6
  fi
  EXY_SHA=$(docker exec "$(ctr_exyonq)" sha256sum /usr/local/bin/exyonq | awk '{print $1}')
  echo "EXYONQ_BINARY_SHA256=$EXY_SHA" | tee "$EV/meta/exyonq_sha.txt"
}

set_framing_arm() {
  local arm=$1
  local val
  case "$arm" in
    A0) val="cap034_chunked_close" ;;
    A1) val="safe_known_length_keepalive" ;;
    *) log "unknown arm $arm"; exit 2 ;;
  esac
  docker exec "$(ctr_exyonq)" sh -c "grep -q EXYONQ_PROXY_WIRE_DOWNSTREAM_FRAMING /proc/1/environ 2>/dev/null || true"
  cat >"$EV/meta/compose-arm.yml" <<EOF
services:
  exyonq:
    environment:
      EXYONQ_PROXY_WIRE_DOWNSTREAM_FRAMING: "$val"
EOF
  compose up -d --no-deps --force-recreate exyonq
  sleep 6
  if [[ -f "$EXY_BIN" ]]; then
    docker cp "$EXY_BIN" "$(ctr_exyonq):/usr/local/bin/exyonq"
    docker restart "$(ctr_exyonq)" >/dev/null
    sleep 6
  fi
  echo "ARM=$arm FRAMING=$val" | tee -a "$EV/meta/arm_log.txt"
}

wire_probe() {
  local arm=$1
  local u
  u=$(url_exyonq)
  compose exec -T bench-runner curl -sS -m 10 -D "/tmp/${arm}-headers.txt" -o /tmp/${arm}-body.bin \
    --http1.1 "$u" | tee "$EV/wire/${arm}_curl_meta.txt" || true
  compose exec -T bench-runner cat "/tmp/${arm}-headers.txt" | tee "$EV/wire/${arm}_response_headers.txt"
  compose exec -T bench-runner wc -c "/tmp/${arm}-body.bin" | tee "$EV/wire/${arm}_body_bytes.txt"
}

run_rep() {
  local arm=$1 rep=$2 order=$3
  local c url statsjson
  c=$(ctr_exyonq)
  url=$(url_exyonq)
  statsjson="$EV/stats/${arm}-rep${rep}.cpu.txt"
  compose exec -T bench-runner rewrk -c "$CONCURRENCY" -d "${WARMUP_SEC}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 || true
  : >"$statsjson"
  timeout $((MEASURE_SEC + 15)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$statsjson" 2>/dev/null &
  local sp=$!
  sleep 0.3
  compose exec -T bench-runner timeout $((MEASURE_SEC + 20)) rewrk -c "$CONCURRENCY" -d "${MEASURE_SEC}s" -t "$THREADS" -h "$url" --json \
    >"$EV/runs/${arm}-rep${rep}.json"
  compose exec -T bench-runner timeout 90 rewrk -c "$CONCURRENCY" -d "${MEASURE_SEC}s" -t "$THREADS" -h "$url" --pct 2>&1 \
    | tee "$EV/runs/${arm}-rep${rep}.pct.txt" || true
  wait "$sp" 2>/dev/null || true
  echo "order=$order arm=$arm rep=$rep" >>"$EV/meta/run_order.txt"
}

perf_stat_rep() {
  local arm=$1 rep=$2
  local c url
  c=$(ctr_exyonq)
  url=$(url_exyonq)
  compose exec -T bench-runner rewrk -c "$CONCURRENCY" -d "${WARMUP_SEC}s" -t "$THREADS" -h "$url" >/dev/null 2>&1 || true
  docker exec "$c" sh -c "command -v perf >/dev/null && perf stat -e cycles,instructions,task-clock,context-switches,cpu-migrations,cache-misses,page-faults \
    -o /tmp/perf-${arm}-${rep}.txt -- timeout $((MEASURE_SEC + 5)) rewrk -c $CONCURRENCY -d ${MEASURE_SEC}s -t $THREADS -h $url" 2>/dev/null || true
  docker cp "$c:/tmp/perf-${arm}-${rep}.txt" "$EV/perf-stat/${arm}-rep${rep}.txt" 2>/dev/null || echo "PERF_NOT_AVAILABLE" >"$EV/perf-stat/${arm}-rep${rep}.txt"
}

connection_metrics() {
  local arm=$1
  local c
  c=$(ctr_exyonq)
  docker exec "$c" sh -c 'ss -s 2>/dev/null || netstat -s 2>/dev/null' | tee "$EV/connection/${arm}_ss_s.txt" || true
}

summarize() {
  python3 - "$EV" "$REPS" <<'PY' | tee "$EV/reports/terminal_summary.txt"
import json, re, statistics, sys
from pathlib import Path

ev = Path(sys.argv[1])
reps = int(sys.argv[2])
ansi = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")

def med(xs):
    return statistics.median(xs) if xs else None

def cv(xs):
    if len(xs) < 2:
        return None
    m = statistics.mean(xs)
    return statistics.stdev(xs) / m if m else None

def pct_delta(a, b):
    if a is None or b is None or a == 0:
        return None
    return (b - a) / a * 100.0

def load_arm(arm):
    rps, p50, p95, p99, cpus, rss = [], [], [], [], [], []
    for rep in range(1, reps + 1):
        jpath = ev / "runs" / f"{arm}-rep{rep}.json"
        if jpath.exists():
            j = json.loads(jpath.read_text())
            rps.append(float(j.get("requests_avg") or 0))
        ppath = ev / "runs" / f"{arm}-rep{rep}.pct.txt"
        if ppath.exists():
            t = ppath.read_text(errors="replace")
            for p, lst in (("50", p50), ("95", p95), ("99", p99)):
                m = re.search(rf"\|\s*{p}%\s*\|\s*([\d.]+)ms", t)
                if m:
                    lst.append(float(m.group(1)))
        spath = ev / "stats" / f"{arm}-rep{rep}.cpu.txt"
        if spath.exists():
            for ln in spath.read_text(errors="replace").splitlines():
                ln = ansi.sub("", ln).strip()
                m2 = re.search(r"([\d.]+)%\s+(\d+(?:\.\d+)?)(MiB|GiB|KiB)", ln)
                if m2:
                    cpus.append(float(m2.group(1)))
                    val = float(m2.group(2))
                    mul = {"KiB": 1/1024, "MiB": 1, "GiB": 1024}[m2.group(3)]
                    rss.append(val * mul)
    return {
        "rps_median": med(rps), "rps_cv": cv(rps),
        "p50": med(p50), "p95": med(p95), "p99": med(p99),
        "cpu_avg": med(cpus), "rss_avg": med(rss),
    }

a0, a1 = load_arm("A0"), load_arm("A1")
rps_d = pct_delta(a0["rps_median"], a1["rps_median"])
p95_d = pct_delta(a0["p95"], a1["p95"])
p99_d = pct_delta(a0["p99"], a1["p99"])

print(f"A0_RPS_MEDIAN={a0['rps_median']}")
print(f"A1_RPS_MEDIAN={a1['rps_median']}")
print(f"A1_VS_A0_RPS_DELTA_PERCENT={rps_d}")
print(f"A0_RPS_CV={a0['rps_cv']}")
print(f"A1_RPS_CV={a1['rps_cv']}")
print(f"A0_P95={a0['p95']}")
print(f"A1_P95={a1['p95']}")
print(f"A0_P99={a0['p99']}")
print(f"A1_P99={a1['p99']}")
print(f"P95_DELTA_PERCENT={p95_d}")
print(f"P99_DELTA_PERCENT={p99_d}")

gain = rps_d or 0
if gain >= 12:
    case = "AB-C34-A"
elif gain >= 3:
    case = "AB-C34-B"
else:
    case = "AB-C34-C"
print(f"CASE={case}")
PY
}

main() {
  write_meta
  ensure_stack
  : >"$EV/meta/run_order.txt"
  local order=0
  for round in $(seq 1 "$REPS"); do
    for arm in A0 A1; do
      order=$((order + 1))
      log "round=$round arm=$arm order=$order"
      set_framing_arm "$arm"
      wire_probe "$arm"
      run_rep "$arm" "$round" "$order"
      perf_stat_rep "$arm" "$round"
      connection_metrics "$arm"
    done
  done
  summarize
  log "done EV=$EV"
}

main "$@"
