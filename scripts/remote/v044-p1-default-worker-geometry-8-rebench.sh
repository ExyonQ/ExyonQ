#!/usr/bin/env bash
# V044_P1_DEFAULT_WORKER_GEOMETRY_8 — rebuild + P1/P2/P3 live three-way.
# PRODUCT mutation already in sources. No EPOLL_POOL/WORKER pin.
set -euo pipefail

WS="${V044_P1_WS:-/root/exyonq-v044-p1-seal-06742e1b}"
TS="${V044_GEOM8_TS:-$(date -u +%Y%m%dT%H%M%SZ)}"
EV="${V044_GEOM8_EV:-$WS/.exyonq-local-evidence/v044-p1-default-worker-geometry-8-$TS}"
FULL_COMPOSE="$WS/benchmarks/docker/docker-compose.bench.yml"
OVER="$WS/benchmarks/docker/docker-compose.p1-authoritative.yml"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
IMAGE="${P1_EXYONQ_IMAGE:-v044p1-geom8-exyonq}"
PRODUCT_COMMIT="${V044_PRODUCT_COMMIT:-ade997a362e60d1dc6eafc984da454930600845a}"
WARMUP_SEC=20
MEASURE_SEC=30
REPS=5

mkdir -p "$EV"/{meta,build,p1,p2,p3,reports}
cd "$WS"
log() { echo "[geom8] $(date -u +%H:%M:%S) $*" | tee -a "$EV/orchestrator.log"; }
compose() { docker compose -f "$FULL_COMPOSE" -f "$OVER" -p "$PROJECT" --profile bench "$@"; }

{
  echo "WIP=V044_P1_DEFAULT_WORKER_GEOMETRY_8"
  echo "PRODUCT_COMMIT=$PRODUCT_COMMIT"
  echo "IMAGE=$IMAGE"
  echo "HOST=$(hostname) ARCH=$(uname -m) NPROC=$(nproc)"
  echo "TS=$TS EV=$EV"
  echo "WORKER_ENV_POLICY=unset_derive_from_available_parallelism"
} | tee "$EV/meta/authority.txt"

# Ensure host env does not re-pin Cap067
unset EXYONQ_WORKER_THREADS EXYONQ_ACCEPT_WORKERS EXYONQ_EPOLL_POOL_THREADS || true
export P1_EXYONQ_IMAGE="$IMAGE"

log "docker build $IMAGE"
export DOCKER_BUILDKIT=1
docker build -f benchmarks/docker/Dockerfile.exyonq -t "${IMAGE}-base" . \
  >"$EV/build/docker-build.log" 2>&1
if [[ -f benchmarks/docker/Dockerfile.exyonq-identity-overlay ]]; then
  docker build -f benchmarks/docker/Dockerfile.exyonq-identity-overlay \
    --build-arg "BASE_IMAGE=${IMAGE}-base" -t "$IMAGE" . \
    >>"$EV/build/docker-build.log" 2>&1 || docker tag "${IMAGE}-base" "$IMAGE"
else
  docker tag "${IMAGE}-base" "$IMAGE"
fi
SHA=$(docker run --rm --entrypoint sha256sum "$IMAGE" /usr/local/bin/exyonq | awk '{print $1}')
echo "EXYONQ_BINARY_SHA256=$SHA" | tee "$EV/meta/exyonq_sha.txt"

# Recreate exyonq with NO worker pins, cpuset 0-7
cat > "$EV/meta/compose-geom8.yml" <<EOF
services:
  exyonq:
    image: ${IMAGE}
    cpuset: "0-7"
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EDGE_STATIC: "1"
      # intentionally omit WORKER/ACCEPT/EPOLL — product defaults
EOF

compose -f "$EV/meta/compose-geom8.yml" up -d --force-recreate --no-deps exyonq
compose up -d --no-deps nginx-stable openlitespeed-latest upstream 2>/dev/null || true
compose --profile bench up -d --no-deps bench-runner 2>/dev/null || true
sleep 10
for tag in nginx ols; do
  c=$(case $tag in nginx) echo ${PROJECT}-nginx-stable-1;; ols) echo ${PROJECT}-openlitespeed-latest-1;; esac)
  docker update --cpuset-cpus "0-7" "$c" >/dev/null || true
done

# Prove geometry
docker logs "${PROJECT}-exyonq-1" 2>&1 | egrep -i "listening|keepalive pool geometry|workers" | tee "$EV/meta/startup_geometry.txt" || true
docker exec "${PROJECT}-exyonq-1" sh -c 'env | egrep "WORKER|ACCEPT|EPOLL" | sort; nproc' | tee "$EV/meta/container_env.txt"

run_profile() {
  local profile=$1 path=$2 outdir=$3
  mkdir -p "$outdir"/{runs,stats,pct}
  log "profile=$profile path=$path"
  for tag in exyonq nginx ols; do
    local c url
    case $tag in
      exyonq) c=${PROJECT}-exyonq-1; url="http://exyonq:8080${path}" ;;
      nginx) c=${PROJECT}-nginx-stable-1; url="http://nginx-stable:8080${path}" ;;
      ols) c=${PROJECT}-openlitespeed-latest-1; url="http://openlitespeed-latest:8088${path}" ;;
    esac
    : >"$outdir/stats/${tag}_rps.txt"
    for rep in $(seq 1 "$REPS"); do
      log "$profile $tag rep $rep/$REPS"
      local statsjson="$outdir/stats/${tag}-rep${rep}.cpu.txt"
      : >"$statsjson"
      timeout $((MEASURE_SEC + WARMUP_SEC + 25)) docker stats --format '{{.CPUPerc}} {{.MemUsage}}' "$c" >"$statsjson" 2>/dev/null &
      local sp=$!
      compose exec -T bench-runner rewrk -c 100 -d "${WARMUP_SEC}s" -t 2 -h "$url" >/dev/null 2>&1 || true
      compose exec -T bench-runner rewrk -c 100 -d "${MEASURE_SEC}s" -t 2 -h "$url" --json \
        >"$outdir/runs/${tag}-rep${rep}.json"
      wait "$sp" 2>/dev/null || true
      python3 -c "import json; print(json.load(open('$outdir/runs/${tag}-rep${rep}.json')).get('requests_avg'))" \
        | tee -a "$outdir/stats/${tag}_rps.txt"
      sleep 3
    done
    compose exec -T bench-runner bash -lc "rewrk -c 100 -d ${MEASURE_SEC}s -t 2 -h $url --pct 2>&1" \
      | tee "$outdir/pct/${tag}.txt" || true
  done
  python3 - "$outdir" "$profile" <<'PY' | tee "$outdir/summary.txt"
import json,re,statistics,sys
from pathlib import Path
d=Path(sys.argv[1]); profile=sys.argv[2]
ansi=re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]")
def med(xs): return statistics.median(xs) if xs else None
def cv(xs):
    if not xs or len(xs)<2: return None
    m=statistics.mean(xs); return (statistics.stdev(xs)/m) if m else None
out={}
for tag in ("exyonq","nginx","ols"):
    rps=[float(x) for x in (d/"stats"/f"{tag}_rps.txt").read_text().split()]
    cpus,rss=[],[]
    for p in sorted((d/"stats").glob(f"{tag}-rep*.cpu.txt")):
        for ln in p.read_text(errors="replace").splitlines():
            ln=ansi.sub("",ln).strip()
            m=re.search(r"([\d.]+)%\s+(\d+(?:\.\d+)?)(MiB|GiB|KiB)", ln)
            if not m: continue
            cpus.append(float(m.group(1)))
            val=float(m.group(2)); unit=m.group(3)
            mul={"KiB":1/1024,"MiB":1,"GiB":1024}[unit]
            rss.append(val*mul)
    rps_m=med(rps); cpu_m=med(cpus); rss_m=med(rss)
    core_ms=(cpu_m/100*8*1000/rps_m) if cpu_m and rps_m else None
    pct={}
    ptxt=(d/"pct"/f"{tag}.txt").read_text(errors="replace") if (d/"pct"/f"{tag}.txt").exists() else ""
    for p in ("50","95","99"):
        m=re.search(rf"\|\s*{p}%\s*\|\s*([\d.]+)ms", ptxt)
        if m: pct[f"p{p}"]=float(m.group(1))
    out[tag]={"rps":rps_m,"rps_runs":rps,"cv":cv(rps),"cpu_pct":cpu_m,"rss_mib":rss_m,"core_ms":core_ms,**pct}
    print(f"{profile}_{tag}_RPS={rps_m}")
    print(f"{profile}_{tag}_CV={cv(rps)}")
    print(f"{profile}_{tag}_CPU_PCT={cpu_m}")
    print(f"{profile}_{tag}_CORE_MS={core_ms}")
    print(f"{profile}_{tag}_RSS={rss_m}")
    for k,v in pct.items(): print(f"{profile}_{tag}_{k.upper()}={v}")
(d/"summary.json").write_text(json.dumps(out,indent=2)+"\n")
# winners
def winner(key, lower_better=False):
    vals={t:out[t].get(key) for t in out if out[t].get(key) is not None}
    if not vals: return "NOT_MEASURED"
    if lower_better: return min(vals, key=vals.get)
    return max(vals, key=vals.get)
print(f"{profile}_WINNER_RPS={winner('rps')}")
print(f"{profile}_WINNER_CPU={winner('core_ms', True)}")
print(f"{profile}_WINNER_MEMORY={winner('rss_mib', True)}")
print(f"{profile}_WINNER_P50={winner('p50', True)}")
print(f"{profile}_WINNER_P95={winner('p95', True)}")
print(f"{profile}_WINNER_P99={winner('p99', True)}")
PY
}

# HTTP probe
for path in /site/1k.bin /site/64k.bin /site/1m.bin; do
  for u in "http://exyonq:8080${path}" "http://nginx-stable:8080${path}" "http://openlitespeed-latest:8088${path}"; do
    compose exec -T bench-runner curl -sf -o /dev/null -w "%{http_code} %{size_download} $u\n" "$u" | tee -a "$EV/meta/probes.txt"
  done
done

run_profile P1 /site/1k.bin "$EV/p1"
run_profile P2 /site/64k.bin "$EV/p2"
run_profile P3 /site/1m.bin "$EV/p3"

# restore note
echo "DONE $(date -u +%Y%m%dT%H%M%SZ)" | tee "$EV/reports/done.txt"
log "COMPLETE EV=$EV SHA=$SHA"
echo "$EV"
