#!/usr/bin/env bash
# Cap067 TCP_NODELAY skb A/B on the shared host kernel.
# Development evidence only. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
# Does not change product default (NODELAY stays ON unless this env is 0).
set -euo pipefail

ROOT="${NODELAY_ROOT:-/root/exyonq-nodelay-ab}"
PROJECT="${COMPOSE_PROJECT:-nodelayab}"
EV="${NODELAY_EV:-$ROOT/.exyonq-local-evidence/tcp-nodelay-skb-$(date -u +%Y%m%dT%H%M%SZ)}"
COMPOSE="$EV/compose.yml"
NET="${PROJECT}_default"
mkdir -p "$EV"
log(){ echo "[nodelay $(date -u +%H:%M:%S)] $*" | tee -a "$EV/orchestrator.log"; }

log "host kernel $(uname -r) $(uname -m)"
echo "KERNEL=$(uname -r)" | tee "$EV/kernel.txt"

# Cap cargo parallelism so an 8-core/15G box does not OOM the release build.
sed -i 's/cargo build --release/CARGO_BUILD_JOBS=4 cargo build --release/g' \
  "$ROOT/benchmarks/docker/Dockerfile.exyonq"

cat >"$COMPOSE" <<EOF
name: ${PROJECT}
services:
  upstream:
    build:
      context: ${ROOT}
      dockerfile: benchmarks/docker/Dockerfile.upstream
    healthcheck:
      test: ["CMD", "curl", "-sf", "http://127.0.0.1:9000/health"]
      interval: 5s
      timeout: 3s
      retries: 12
      start_period: 5s
  exyonq:
    image: nodelayab-exyonq:local
    build:
      context: ${ROOT}
      dockerfile: benchmarks/docker/Dockerfile.exyonq
    depends_on:
      upstream:
        condition: service_healthy
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_EPOLL_LISTEN: "1"
      EXYONQ_EPOLL_STATIC: "1"
      EXYONQ_ACCEPT_WORKERS: "8"
      EXYONQ_WORKER_THREADS: "8"
      EXYONQ_EPOLL_POOL_THREADS: "8"
      EXYONQ_LARGE_BODY_TCP_CORK: "1"
      EXYONQ_SENDFILE_CHUNK: "131072"
      EXYONQ_TCP_NODELAY: "\${EXYONQ_TCP_NODELAY:-1}"
    volumes:
      - ${ROOT}/benchmarks/configs/exyonq/bench.toml:/bench/bench.toml:ro
      - ${ROOT}/benchmarks/scenarios/payloads/www:/bench/www:ro
      - ${ROOT}/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
  nginx:
    image: nginx:1.30.3-alpine
    volumes:
      - ${ROOT}/benchmarks/configs/nginx/nginx.conf:/etc/nginx/nginx.conf:ro
      - ${ROOT}/benchmarks/scenarios/payloads/www:/bench/www:ro
      - ${ROOT}/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
EOF

compose() { docker compose -f "$COMPOSE" -p "$PROJECT" "$@"; }

log "docker build exyonq+upstream"
compose build exyonq upstream 2>&1 | tee "$EV/docker-build.log"

export EXYONQ_TCP_NODELAY=1
compose up -d --force-recreate upstream exyonq nginx
docker update --cpuset-cpus 0-7 "${PROJECT}-exyonq-1" "${PROJECT}-nginx-1" >/dev/null || true

wait_health() {
  local cname=$1 url=$2
  for i in $(seq 1 60); do
    if docker exec "$cname" sh -c "command -v curl >/dev/null && curl -sf $url >/dev/null" \
      || docker exec "$cname" sh -c "command -v wget >/dev/null && wget -qO- $url | grep -q ok"; then
      log "$cname healthy try $i"
      return 0
    fi
    sleep 2
  done
  log "FAIL health $cname"
  docker logs "$cname" --tail 40 | tee "$EV/${cname}.log" || true
  exit 2
}
wait_health "${PROJECT}-exyonq-1" http://127.0.0.1:8080/health
wait_health "${PROJECT}-nginx-1" http://127.0.0.1:8080/health

log "prepare wrk sidecar"
docker rm -f nodelay-wrk >/dev/null 2>&1 || true
docker run -d --name nodelay-wrk --network "$NET" --entrypoint sleep alpine:3.20 36000 >/dev/null
docker exec nodelay-wrk apk add --no-cache wrk iproute2 ethtool >/dev/null

offload() {
  local cname=$1
  log "offload $cname"
  docker run --rm --network "container:${cname}" alpine:3.20 \
    sh -c 'apk add --no-cache ethtool >/dev/null && echo KERNEL=$(uname -r) && ethtool -k eth0 | grep -E "tcp-segmentation-offload|generic-segmentation-offload|generic-receive-offload|tx-checksumming|rx-checksumming"' \
    | tee "$EV/offload-${cname}.txt" || true
}
offload "${PROJECT}-exyonq-1"
offload "${PROJECT}-nginx-1"

parse_window() {
  local tag=$1
  python3 - "$EV" "$tag" <<'PY'
import re, sys
from pathlib import Path
ev, tag = Path(sys.argv[1]), sys.argv[2]

def fields(path, section):
    lines = [ln.split() for ln in Path(path).read_text().splitlines() if ln.split()[:1] == [section]]
    if len(lines) < 2:
        raise SystemExit(f"missing {section} in {path}")
    hdr, val = lines[0][1:], lines[1][1:]
    return {k: int(v) for k, v in zip(hdr, val)}

def usage(path):
    for ln in Path(path).read_text().splitlines():
        if ln.startswith("usage_usec"):
            return int(ln.split()[1])
    raise SystemExit(f"no usage_usec in {path}")

snmp0, snmp1 = fields(ev/f"{tag}.snmp0", "Tcp:"), fields(ev/f"{tag}.snmp1", "Tcp:")
ext0, ext1 = fields(ev/f"{tag}.net0", "TcpExt:"), fields(ev/f"{tag}.net1", "TcpExt:")
cpu0, cpu1 = usage(ev/f"{tag}.cpu0"), usage(ev/f"{tag}.cpu1")
wrk = (ev/f"{tag}.wrk").read_text()
mreq = re.search(r"(\d+) requests in", wrk)
mrps = re.search(r"Requests/sec:\s+([0-9.]+)", wrk)
if not mreq or not mrps:
    raise SystemExit(f"wrk parse fail {tag}\n{wrk}")
reqs = int(mreq.group(1))
rps = float(mrps.group(1))
out = snmp1["OutSegs"] - snmp0["OutSegs"]
orig = ext1["TCPOrigDataSent"] - ext0["TCPOrigDataSent"]
retrans = snmp1.get("RetransSegs", 0) - snmp0.get("RetransSegs", 0)
cms = ((cpu1 - cpu0) / 1e6) / reqs * 1000.0
ss = (ev/f"{tag}.ss").read_text().strip() if (ev/f"{tag}.ss").exists() else ""
print(
    f"{tag}\treqs={reqs}\trps={rps:.1f}\tOutSegs={out}\tOutSegs/req={out/reqs:.2f}\t"
    f"TCPOrigDataSent={orig}\torig/req={orig/reqs:.2f}\tretrans={retrans}\t"
    f"core_ms={cms:.3f}\tss={ss}"
)
PY
}

measure() {
  local tag=$1 cname=$2 host=$3
  log "MEASURE $tag $host"
  docker exec nodelay-wrk wrk -t2 -c100 -d 8s --latency "http://${host}:8080/site/1m.bin" >/dev/null
  docker exec "$cname" cat /proc/net/snmp >"$EV/${tag}.snmp0"
  docker exec "$cname" cat /proc/net/netstat >"$EV/${tag}.net0"
  docker exec "$cname" cat /sys/fs/cgroup/cpu.stat >"$EV/${tag}.cpu0"
  docker exec nodelay-wrk wrk -t2 -c100 -d 20s --latency "http://${host}:8080/site/1m.bin" >"$EV/${tag}.wrk" &
  local wp=$!
  sleep 4
  docker run --rm --network "container:${cname}" alpine:3.20 \
    sh -c 'apk add --no-cache iproute2 >/dev/null && ss -tin | awk "/ESTAB/{e++} /nodelay/{n++} END{printf \"estab=%d nodelay=%d\\n\", e+0, n+0}"' \
    >"$EV/${tag}.ss" || echo "ss_fail" >"$EV/${tag}.ss"
  wait "$wp"
  docker exec "$cname" cat /proc/net/snmp >"$EV/${tag}.snmp1"
  docker exec "$cname" cat /proc/net/netstat >"$EV/${tag}.net1"
  docker exec "$cname" cat /sys/fs/cgroup/cpu.stat >"$EV/${tag}.cpu1"
  parse_window "$tag" | tee -a "$EV/summary.tsv"
}

docker exec "${PROJECT}-exyonq-1" env | grep -E 'EXYONQ_(TCP_NODELAY|EPOLL_LISTEN|SENDFILE|LARGE_BODY|ACCEPT|WORKER)' | sort \
  | tee "$EV/env-nodelay1.txt"

measure ex-nodelay-on "${PROJECT}-exyonq-1" exyonq
measure nginx-stable "${PROJECT}-nginx-1" nginx

log "recreate exyonq TCP_NODELAY=0"
export EXYONQ_TCP_NODELAY=0
compose up -d --no-deps --force-recreate exyonq
docker update --cpuset-cpus 0-7 "${PROJECT}-exyonq-1" >/dev/null || true
wait_health "${PROJECT}-exyonq-1" http://127.0.0.1:8080/health
docker exec "${PROJECT}-exyonq-1" env | grep EXYONQ_TCP_NODELAY | tee "$EV/env-nodelay0.txt"
measure ex-nodelay-off "${PROJECT}-exyonq-1" exyonq

log "DONE"
cat "$EV/summary.tsv"
echo "EV=$EV"
