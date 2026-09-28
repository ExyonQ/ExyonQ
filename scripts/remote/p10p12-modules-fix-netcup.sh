#!/usr/bin/env bash
# Fix P10/P12 label inflation: remasure with bench-modules.toml
# (ratelimit ON high-ceiling + metrics ON). DEV NOT official.
set -euo pipefail

ROOT=/root/exyonq-nodelay-ab
COMPOSE="$ROOT/.exyonq-local-evidence/tcp-nodelay-skb-20260919T123034Z/compose.yml"
TS=$(date -u +%Y%m%dT%H%M%SZ)
EV="$ROOT/.exyonq-local-evidence/p10p12-modules-fix-$TS"
mkdir -p "$EV"
echo "$EV" > /tmp/p10p12-modules-ev.path
export EV

{
  echo "DEV_NOT_OFFICIAL=1"
  echo "purpose=fix P10/P12 scope laundering — real modules config (EPOLL off: Hyper modules path)"
  echo "ts_utc=$TS"
  uptime
} | tee "$EV/meta.txt"

for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik; do
  docker pause "$c" 2>/dev/null || true
done

# Ensure host modules toml is current (metrics enabled=true)
grep -A3 'modules.metrics' "$ROOT/benchmarks/configs/exyonq/bench-modules.toml" | tee -a "$EV/meta.txt"
grep -A4 'modules.ratelimit' "$ROOT/benchmarks/configs/exyonq/bench-modules.toml" | tee -a "$EV/meta.txt"

cat >"$EV/overlay-modules.yml" <<'OYAML'
services:
  exyonq:
    environment:
      EXYONQ_CONFIG: /bench/bench-modules.toml
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_EPOLL_POOL_THREADS: "4"
      EXYONQ_EPOLL_LISTEN: "0"
      EXYONQ_EPOLL_STATIC: "0"
      EXYONQ_EPOLL_SENDFILE: "0"
      EXYONQ_STATIC_ENCODING_CACHE: "1"
      EXYONQ_STATIC_ENCODING_CACHE_DIR: "/tmp/exyonq-static-encoding"
    volumes:
      - /root/exyonq-nodelay-ab/benchmarks/configs/exyonq/bench.toml:/bench/bench.toml:ro
      - /root/exyonq-nodelay-ab/benchmarks/configs/exyonq/bench-modules.toml:/bench/bench-modules.toml:ro
      - /root/exyonq-nodelay-ab/benchmarks/scenarios/payloads/www:/bench/www:ro
      - /root/exyonq-nodelay-ab/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
OYAML

docker compose -f "$COMPOSE" -f "$EV/overlay-modules.yml" -p nodelayab up -d --no-deps --force-recreate exyonq
health(){ for i in $(seq 1 90); do docker exec nodelayab-exyonq-1 curl -sf http://127.0.0.1:8080/health >/dev/null && return 0; sleep 1; done; return 1; }
health

docker exec nodelayab-exyonq-1 printenv | grep -E 'CONFIG|EPOLL|ENCODING' | tee "$EV/env.txt"
docker exec nodelayab-exyonq-1 sha256sum /usr/local/bin/exyonq /bench/bench-modules.toml | tee "$EV/sha.txt"
docker exec nodelayab-exyonq-1 sh -c 'grep -A3 modules.metrics /bench/bench-modules.toml; grep -A4 modules.ratelimit /bench/bench-modules.toml' | tee "$EV/config-in-container.txt"

# Causal: metrics endpoint must be live with bearer (P12 proof)
docker exec nodelay-wrk sh -c 'wget -T 5 -S -O /tmp/met http://exyonq:8080/metrics --header="Authorization: Bearer bench-p12-metrics-token" 2>&1 | head -20; echo body=$(wc -c </tmp/met); head -5 /tmp/met' | tee "$EV/causal-metrics.txt"
# Causal: /api/ still 1024
docker exec nodelay-wrk sh -c 'wget -T 5 -S -O /tmp/api http://exyonq:8080/api/ 2>&1 | head -12; echo body=$(wc -c </tmp/api)' | tee "$EV/causal-api.txt"

# Fail closed if metrics not actually served
if ! grep -qE 'HTTP/1.1 200|HTTP/1.0 200' "$EV/causal-metrics.txt"; then
  echo "FAIL: /metrics not 200 — modules config not engaged" | tee -a "$EV/meta.txt"
  exit 1
fi
if ! grep -qiE 'exyonq_|prometheus|HELP |TYPE ' "$EV/causal-metrics.txt"; then
  # body may be in separate file inside container output — check size at least
  if ! grep -qE 'body=[1-9]' "$EV/causal-metrics.txt"; then
    echo "FAIL: /metrics empty" | tee -a "$EV/meta.txt"
    exit 1
  fi
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

# P4 control WITH modules config (same binary/config as P10/P12 — isolates label)
# Plus P10 (=ratelimit path) and P12 (=metrics path) — same URL, modules ON
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

# After load: metrics still serving and growing
docker exec nodelay-wrk sh -c 'wget -T 5 -q -O /tmp/met2 http://exyonq:8080/metrics --header="Authorization: Bearer bench-p12-metrics-token"; wc -c </tmp/met2; grep -c exyonq_ /tmp/met2 || true' | tee "$EV/causal-metrics-after.txt"

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
print("FIX: P10/P12 under EXYONQ_CONFIG=/bench/bench-modules.toml")
print("  ratelimit enabled=true (1e6 rps ceiling); metrics enabled=true")
print("P4 here = same modules config (control), NOT default bench.toml")
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
# ExyonQ delta modules-on P10/P12 vs prior fake P10/P12 (~112k)
prior = {"P10": 111679.0, "P12": 113419.0, "P4_fake": 120054.0}
by = {}
for sc, s, r in rows:
    by.setdefault(sc, {})[s] = r
if "exyonq" in by.get("P4", {}):
    print(f"exyonq P4(modules-config) med={by['P4']['exyonq']:.0f}")
for sc in ("P10", "P12"):
    if "exyonq" in by.get(sc, {}):
        ex = by[sc]["exyonq"]
        print(f"exyonq {sc}(modules-on) med={ex:.0f} vs prior_mislabeled={prior[sc]:.0f} delta={(ex/prior[sc]-1)*100:+.1f}%")
        if "ols" in by[sc]:
            print(f"  vs ols {(ex/by[sc]['ols']-1)*100:+.1f}%")
PY

# Restore default bench.toml overlay (encoding cache Cap067) so stack stays usable
cat >"$EV/overlay-restore.yml" <<'OYAML'
services:
  exyonq:
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_EPOLL_POOL_THREADS: "4"
      EXYONQ_EPOLL_LISTEN: "0"
      EXYONQ_EPOLL_STATIC: "0"
      EXYONQ_EPOLL_SENDFILE: "0"
      EXYONQ_STATIC_ENCODING_CACHE: "1"
      EXYONQ_STATIC_ENCODING_CACHE_DIR: "/tmp/exyonq-static-encoding"
OYAML
docker compose -f "$COMPOSE" -f "$EV/overlay-restore.yml" -p nodelayab up -d --no-deps --force-recreate exyonq
health || true

for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik; do
  docker unpause "$c" 2>/dev/null || true
done
echo DONE | tee "$EV/DONE" | tee -a "$EV/summary.txt"
cat "$EV/SCOREBOARD.txt"
