#!/usr/bin/env bash
# Remasure P10/P12 with bench-modules.toml AND Cap067 EPOLL ON.
# Requires /metrics Hyper-handoff fix (824b145 / hang-fix suite).
# DEV_NOT_OFFICIAL=1 — not rule-104 official.
set -euo pipefail

ROOT=/root/exyonq-nodelay-ab
COMPOSE_DIR="$ROOT/.exyonq-local-evidence/tcp-nodelay-skb-20260919T123034Z"
COMPOSE="$COMPOSE_DIR/compose.yml"
# Survive host rsync --delete wiping evidence: recreate base compose if missing.
if [[ ! -f "$COMPOSE" ]]; then
  mkdir -p "$COMPOSE_DIR"
  cat >"$COMPOSE" <<EOF
name: nodelayab
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
      EXYONQ_EPOLL_SENDFILE: "1"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_EPOLL_POOL_THREADS: "4"
      EXYONQ_LARGE_BODY_TCP_CORK: "1"
      EXYONQ_SENDFILE_CHUNK: "131072"
      EXYONQ_TCP_NODELAY: "\${EXYONQ_TCP_NODELAY:-1}"
      EXYONQ_STATIC_ENCODING_CACHE: "1"
      EXYONQ_STATIC_ENCODING_CACHE_DIR: "/tmp/exyonq-static-encoding"
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
fi
TS=$(date -u +%Y%m%dT%H%M%SZ)
EV="$ROOT/.exyonq-local-evidence/p10p12-modules-epoll-on-$TS"
mkdir -p "$EV"
echo "$EV" > /tmp/p10p12-modules-epoll-ev.path
export EV

{
  echo "DEV_NOT_OFFICIAL=1"
  echo "purpose=P4/P10/P12 remasure post P5-handoff KEEP + AE-gate + wire-cheap modules EPOLL ON"
  echo "commits=5f367bc,07c6fb4,655465c"
  echo "prior_hyper_force=p10p12-modules-epoll-on-20260923T125036Z (~45-50k vs OLS ~120k)"
  echo "prior_wire_cheap=p10p12-wire-cheap-final-20260923T135804Z (~126k vs OLS)"
  echo "prior_ae_gate=p10p12-ae-gate-20260923T145238Z (~136k AE-idle)"
  echo "ts_utc=$TS"
  uptime
} | tee "$EV/meta.txt"

for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik; do
  docker pause "$c" 2>/dev/null || true
done
docker start nodelay-wrk 2>/dev/null || true

grep -A3 'modules.metrics' "$ROOT/benchmarks/configs/exyonq/bench-modules.toml" | tee -a "$EV/meta.txt"
grep -A4 'modules.ratelimit' "$ROOT/benchmarks/configs/exyonq/bench-modules.toml" | tee -a "$EV/meta.txt"

cat >"$EV/overlay-modules-epoll.yml" <<'OYAML'
services:
  exyonq:
    environment:
      EXYONQ_CONFIG: /bench/bench-modules.toml
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_EPOLL_POOL_THREADS: "4"
      EXYONQ_EPOLL_LISTEN: "1"
      EXYONQ_EPOLL_STATIC: "1"
      EXYONQ_EPOLL_SENDFILE: "1"
      EXYONQ_STATIC_ENCODING_CACHE: "1"
      EXYONQ_STATIC_ENCODING_CACHE_DIR: "/tmp/exyonq-static-encoding"
    volumes:
      - /root/exyonq-nodelay-ab/benchmarks/configs/exyonq/bench.toml:/bench/bench.toml:ro
      - /root/exyonq-nodelay-ab/benchmarks/configs/exyonq/bench-modules.toml:/bench/bench-modules.toml:ro
      - /root/exyonq-nodelay-ab/benchmarks/scenarios/payloads/www:/bench/www:ro
      - /root/exyonq-nodelay-ab/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
OYAML

# Rebuild so Netcup tree includes hang-fix eligibility + A2 drain (already synced).
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:${PATH}"
if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
fi
echo "=== docker rebuild exyonq (EPOLL ON + wire-cheap modules) ===" | tee -a "$EV/meta.txt"
docker compose -f "$COMPOSE" -f "$EV/overlay-modules-epoll.yml" -p nodelayab build exyonq 2>&1 | tee "$EV/build.log" | tail -30
docker compose -f "$COMPOSE" -f "$EV/overlay-modules-epoll.yml" -p nodelayab up -d --no-deps --force-recreate exyonq

health(){ for i in $(seq 1 90); do docker exec nodelayab-exyonq-1 curl -sf http://127.0.0.1:8080/health >/dev/null && return 0; sleep 1; done; return 1; }
health

docker exec nodelayab-exyonq-1 printenv | grep -E 'CONFIG|EPOLL|ENCODING' | tee "$EV/env.txt"
docker exec nodelayab-exyonq-1 sha256sum /usr/local/bin/exyonq /bench/bench-modules.toml | tee "$EV/sha.txt"
docker exec nodelayab-exyonq-1 sh -c 'grep -A3 modules.metrics /bench/bench-modules.toml; grep -A4 modules.ratelimit /bench/bench-modules.toml' | tee "$EV/config-in-container.txt"

# Causal: /metrics must complete under EPOLL ON (hang regression gate)
docker exec nodelayab-exyonq-1 sh -c \
  'curl -sS -m 5 -D - -o /tmp/met -H "Authorization: Bearer bench-p12-metrics-token" http://127.0.0.1:8080/metrics; echo EXIT=$?; echo body=$(wc -c </tmp/met); head -5 /tmp/met' \
  | tee "$EV/causal-metrics.txt"
docker exec nodelayab-exyonq-1 sh -c \
  'curl -sS -m 5 -D - -o /tmp/api http://127.0.0.1:8080/api/; echo EXIT=$?; echo body=$(wc -c </tmp/api)' \
  | tee "$EV/causal-api.txt"

# Path oracle: Cap054 counters must move after wire /api/ (wire_record_response).
# Hyper Service must NOT be the only path — metrics scrape stays Hyper (LA-CAP054-008).
docker exec nodelayab-exyonq-1 sh -c \
  'curl -sS -m 5 -H "Authorization: Bearer bench-p12-metrics-token" http://127.0.0.1:8080/metrics | grep -E "exyonq_http_requests_total|exyonq_http_responses" | head -20' \
  | tee "$EV/oracle-metrics-before.txt"
docker exec nodelay-wrk wrk -t2 -c50 -d 3s http://exyonq:8080/api/ >/dev/null 2>&1 || true
docker exec nodelayab-exyonq-1 sh -c \
  'curl -sS -m 5 -H "Authorization: Bearer bench-p12-metrics-token" http://127.0.0.1:8080/metrics | grep -E "exyonq_http_requests_total|exyonq_http_responses" | head -20' \
  | tee "$EV/oracle-metrics-after-wire.txt"
# Stack sample under load: prefer proxy_wire / epoll over hyper::service for /api/
(
  docker exec nodelay-wrk wrk -t2 -c100 -d 4s http://exyonq:8080/api/ >/dev/null 2>&1 &
  WP=$!
  sleep 1
  docker exec nodelayab-exyonq-1 sh -c 'pidof exyonq | tr " " "\n" | head -1' | tee "$EV/oracle-pid.txt"
  PID=$(cat "$EV/oracle-pid.txt")
  if [[ -n "$PID" ]]; then
    docker exec nodelayab-exyonq-1 sh -c "timeout 2 cat /proc/$PID/stack 2>/dev/null || true; ls /proc/$PID/task 2>/dev/null | head -5 | while read t; do echo ===\$t===; timeout 1 cat /proc/$PID/task/\$t/stack 2>/dev/null || true; done" \
      | tee "$EV/oracle-stacks.txt" || true
  fi
  wait "$WP" || true
)

if ! grep -qE 'HTTP/1.1 200|HTTP/1.0 200' "$EV/causal-metrics.txt"; then
  echo "FAIL: /metrics not 200 under Cap067 EPOLL+modules — abort remasure" | tee -a "$EV/meta.txt"
  exit 1
fi
if ! grep -qE 'body=[1-9]' "$EV/causal-metrics.txt"; then
  echo "FAIL: /metrics empty" | tee -a "$EV/meta.txt"
  exit 1
fi
if ! grep -qE 'HTTP/1.1 200|HTTP/1.0 200' "$EV/causal-api.txt"; then
  echo "FAIL: /api not 200" | tee -a "$EV/meta.txt"
  exit 1
fi

docker exec -i nodelay-wrk sh -c 'cat >/tmp/lat.lua' <<'LUA'
done = function(summary, latency, requests)
  local sec = summary.duration / 1000000
  io.write(string.format("rps %.1f\n", summary.requests / sec))
  io.write(string.format("requests %d\n", summary.requests))
  io.write(string.format("duration_us %d\n", summary.duration))
  io.write(string.format("p50_us %.1f\n", latency:percentile(50)))
  io.write(string.format("p90_us %.1f\n", latency:percentile(90)))
  io.write(string.format("p95_us %.1f\n", latency:percentile(95)))
  io.write(string.format("p99_us %.1f\n", latency:percentile(99)))
  io.write(string.format("errors %d\n", summary.errors.connect + summary.errors.read + summary.errors.write + summary.errors.status + summary.errors.timeout))
end
LUA

declare -A URL CTR
URL[exyonq]=http://exyonq:8080
CTR[exyonq]=nodelayab-exyonq-1
URL[nginx]=http://nginx:8080
CTR[nginx]=nodelayab-nginx-1
URL[ols]=http://nodelay-ols:8088
CTR[ols]=nodelay-ols
URL[haproxy]=http://nodelay-haproxy:8080
CTR[haproxy]=nodelay-haproxy
SERVERS=(exyonq nginx ols haproxy)

cp_usage() {
  docker exec "$1" sh -c 'awk "/^usage_usec /{print \$2}" /sys/fs/cgroup/cpu.stat 2>/dev/null || echo 0'
}

measure() {
  local tag=$1 cname=$2 url=$3 conns=$4
  cp_usage "$cname" >"$EV/${tag}.cpu0" || echo 0 >"$EV/${tag}.cpu0"
  local t0; t0=$(date +%s%N)
  (
    for _ in 1 2 3 4 5 6; do
      docker exec "$cname" sh -c 'cat /sys/fs/cgroup/memory.current 2>/dev/null || echo 0'
      sleep 2
    done
  ) >"$EV/${tag}.mem" &
  local mp=$!
  docker exec nodelay-wrk wrk -t2 -c"$conns" -d 12s -s /tmp/lat.lua "$url" >"$EV/${tag}.wrk" 2>&1 || true
  wait "$mp" || true
  local t1; t1=$(date +%s%N)
  echo $((t1 - t0)) >"$EV/${tag}.wallns"
  cp_usage "$cname" >"$EV/${tag}.cpu1" || echo 0 >"$EV/${tag}.cpu1"
}

: >"$EV/summary.txt"

# Warm
docker exec nodelay-wrk wrk -t2 -c50 -d 3s -s /tmp/lat.lua http://exyonq:8080/api/ >/dev/null 2>&1 || true

# P4 control + P10 (ratelimit) + P12 (metrics) — same URL /api/, modules ON, Cap067 EPOLL ON
for sc in P4 P10 P12; do
  for r in 1 2 3; do
    for s in "${SERVERS[@]}"; do
      tag="${s}-${sc}-r${r}"
      echo "MEASURE $tag" | tee -a "$EV/summary.txt"
      measure "$tag" "${CTR[$s]}" "${URL[$s]}/api/" 100
      awk -v t="$tag" '/^rps /{printf "%s %s\n", t, $2}' "$EV/${tag}.wrk" | tee -a "$EV/summary.txt" || true
    done
  done
done

docker exec nodelayab-exyonq-1 sh -c \
  'curl -sS -m 5 -o /tmp/met2 -H "Authorization: Bearer bench-p12-metrics-token" http://127.0.0.1:8080/metrics; wc -c </tmp/met2; grep -c exyonq_ /tmp/met2 || true' \
  | tee "$EV/causal-metrics-after.txt"

python3 - <<'PY' | tee "$EV/SCOREBOARD.txt"
import os, re, statistics, pathlib
ev = pathlib.Path(os.environ["EV"])
servers = ["exyonq", "nginx", "ols", "haproxy"]
scenarios = ["P4", "P10", "P12"]

def parse_wrk(p):
    t = p.read_text(errors="ignore")
    def f(k):
        m = re.search(rf"^{k} ([0-9.]+)", t, re.M)
        return float(m.group(1)) if m else None
    return {k: f(k) for k in ["rps", "p95_us", "p99_us", "errors"]}

def cpu_cores(tag):
    try:
        u0 = int((ev / f"{tag}.cpu0").read_text().strip().split()[0])
        u1 = int((ev / f"{tag}.cpu1").read_text().strip().split()[0])
        wall = int((ev / f"{tag}.wallns").read_text().strip())
        if wall <= 0:
            return None
        return (u1 - u0) * 1000.0 / wall
    except Exception:
        return None

def rss_mb(tag):
    try:
        vals = [int(x) for x in (ev / f"{tag}.mem").read_text().split() if x.isdigit()]
        if not vals:
            return None
        return statistics.median(vals) / (1024 * 1024)
    except Exception:
        return None

print("DEV evidence — NOT official (rule 104)")
print("REMEASURE: Cap067 EPOLL ON + EXYONQ_CONFIG=/bench/bench-modules.toml")
print("  ratelimit enabled=true (1e6 rps ceiling); metrics enabled=true")
print("  /metrics Hyper-handoff (no hang)")
print(f"EV={ev}")
print()
hdr = f"{'sc':<4} {'server':<8} {'RPS med':>10} {'p95 ms':>8} {'p99 ms':>8} {'CPU c':>7} {'RSS MB':>8} {'err':>6}"
print(hdr)
print("-" * len(hdr))
rows = []
for sc in scenarios:
    for s in servers:
        rps, p95, p99, cpu, rss, err = [], [], [], [], [], []
        for r in (1, 2, 3):
            tag = f"{s}-{sc}-r{r}"
            wp = ev / f"{tag}.wrk"
            if not wp.exists():
                continue
            w = parse_wrk(wp)
            if w["rps"] is not None:
                rps.append(w["rps"])
            if w["p95_us"] is not None:
                p95.append(w["p95_us"] / 1000.0)
            if w["p99_us"] is not None:
                p99.append(w["p99_us"] / 1000.0)
            if w["errors"] is not None:
                err.append(w["errors"])
            c = cpu_cores(tag)
            if c is not None:
                cpu.append(c)
            m = rss_mb(tag)
            if m is not None:
                rss.append(m)
        if not rps:
            print(f"{sc:<4} {s:<8} {'MISSING':>10}")
            continue
        print(
            f"{sc:<4} {s:<8} {statistics.median(rps):10.0f} "
            f"{(statistics.median(p95) if p95 else float('nan')):8.2f} "
            f"{(statistics.median(p99) if p99 else float('nan')):8.2f} "
            f"{(statistics.median(cpu) if cpu else float('nan')):7.2f} "
            f"{(statistics.median(rss) if rss else float('nan')):8.1f} "
            f"{max(err) if err else 0:6.0f}"
        )
        rows.append((sc, s, statistics.median(rps)))

print()
# Compare vs prior EPOLL-off modules remasure
prior_off = {"P4": 45400.0, "P10": 45400.0, "P12": 48000.0}  # approx from 20260922T172617Z
by = {}
for sc, s, r in rows:
    by.setdefault(sc, {})[s] = r
for sc in scenarios:
    if "exyonq" in by.get(sc, {}):
        ex = by[sc]["exyonq"]
        print(f"exyonq {sc}(EPOLL ON + modules) med={ex:.0f}")
        if sc in prior_off:
            print(f"  vs prior EPOLL-off ~{prior_off[sc]:.0f} delta={(ex/prior_off[sc]-1)*100:+.1f}% (approx)")
        if "ols" in by.get(sc, {}):
            print(f"  vs ols {(ex/by[sc]['ols']-1)*100:+.1f}%")
        if "nginx" in by.get(sc, {}):
            print(f"  vs nginx {(ex/by[sc]['nginx']-1)*100:+.1f}%")
PY

# Leave stack on Cap067 EPOLL ON + modules for follow-up probes
touch "$EV/DONE"
echo DONE | tee -a "$EV/summary.txt"
cat "$EV/SCOREBOARD.txt"

for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik; do
  docker unpause "$c" 2>/dev/null || true
done
