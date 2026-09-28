#!/usr/bin/env bash
# DEV P1–P14 rivals matrix (Netcup nodelayab). NOT official (rule 104).
# Servers: exyonq + nginx + ols + haproxy.
# Metrics: RPS, p95, p99, CPU cores, RSS MB.
# wrk -t2 -d12s ×3 interleaved per scenario.
set -euo pipefail

ROOT=/root/exyonq-nodelay-ab
TS=$(date -u +%Y%m%dT%H%M%SZ)
EV="$ROOT/.exyonq-local-evidence/p1p14-rivals-$TS"
mkdir -p "$EV"
echo "$EV" > /tmp/p1p14-rivals-ev.path
export EV

{
  echo "DEV_NOT_OFFICIAL=1"
  echo "policy=104-benchmark-secuencial DEV matrix"
  echo "ts_utc=$TS"
  echo "host=$(hostname)"
  uptime
  free -h | head -2
  df -h / | tail -1
} | tee "$EV/meta.txt"

for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik; do
  docker pause "$c" 2>/dev/null || true
done

if ! docker inspect -f '{{.State.Running}}' nodelay-ols 2>/dev/null | grep -q true; then
  docker start nodelay-ols 2>/dev/null || true
  sleep 2
fi

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

docker exec nodelayab-exyonq-1 printenv | grep -E 'EPOLL|ENCODING|WORKER|ACCEPT|CORK|NODELAY|SENDFILE' | sort | tee "$EV/exyonq.env"
docker exec nodelayab-exyonq-1 sha256sum /usr/local/bin/exyonq | tee "$EV/exyonq.sha"

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

docker exec -i nodelay-wrk sh -c 'cat >/tmp/p5.lua' <<'LUA'
wrk.method = "GET"
wrk.headers["Connection"] = "close"
dofile("/tmp/lat.lua")
LUA

docker exec -i nodelay-wrk sh -c 'cat >/tmp/p7.lua' <<'LUA'
paths = {}
for i=0,99 do paths[#paths+1] = string.format("/site/routes/route%03d.bin", i) end
counter = 0
request = function()
  counter = counter + 1
  return wrk.format(nil, paths[((counter-1) % #paths)+1])
end
dofile("/tmp/lat.lua")
LUA

docker exec -i nodelay-wrk sh -c 'cat >/tmp/p9.lua' <<'LUA'
wrk.method = "GET"
wrk.headers["Accept-Encoding"] = "gzip"
wrk.headers["Connection"] = "keep-alive"
dofile("/tmp/lat.lua")
LUA

cp_usage() {
  docker exec "$1" sh -c 'awk "/^usage_usec /{print \$2}" /sys/fs/cgroup/cpu.stat 2>/dev/null || echo 0'
}

measure() {
  local tag=$1 cname=$2 url=$3 conns=$4 script=$5
  local -a wrk_args=(-t2 -c"$conns" -d 12s -s "${script:-/tmp/lat.lua}")

  cp_usage "$cname" >"$EV/${tag}.cpu0" || echo 0 >"$EV/${tag}.cpu0"
  local t0
  t0=$(date +%s%N)
  (
    for _ in 1 2 3 4 5 6; do
      docker exec "$cname" sh -c 'cat /sys/fs/cgroup/memory.current 2>/dev/null || echo 0'
      sleep 2
    done
  ) >"$EV/${tag}.mem" &
  local mp=$!
  docker exec nodelay-wrk wrk "${wrk_args[@]}" "$url" >"$EV/${tag}.wrk" 2>&1 || true
  wait "$mp" || true
  local t1
  t1=$(date +%s%N)
  echo $((t1 - t0)) >"$EV/${tag}.wallns"
  cp_usage "$cname" >"$EV/${tag}.cpu1" || echo 0 >"$EV/${tag}.cpu1"
}

: >"$EV/skips.txt"
: >"$EV/summary.txt"

echo "SKIP P11 SKIP_NO_TLS_ON_NODELAYAB" | tee -a "$EV/skips.txt"
echo "SKIP P13 SKIP_NO_HTTP3_ON_NODELAYAB" | tee -a "$EV/skips.txt"
echo "SKIP P14 SKIP_NO_SCENARIO" | tee -a "$EV/skips.txt"

# P10/P12 require EXYONQ_CONFIG=/bench/bench-modules.toml (ratelimit+metrics).
# Measuring them as plain /api/ under bench.toml is scope laundering — SKIP here.
# See scripts/remote/p10p12-modules-fix-netcup.sh for the corrected remasure.
echo "SKIP P10 SKIP_NEEDS_MODULES_CONFIG (use p10p12-modules-fix-netcup.sh)" | tee -a "$EV/skips.txt"
echo "SKIP P12 SKIP_NEEDS_MODULES_CONFIG (use p10p12-modules-fix-netcup.sh)" | tee -a "$EV/skips.txt"

SCENARIOS=(
  "P1|/site/1k.bin|100|/tmp/lat.lua"
  "P2|/site/64k.bin|100|/tmp/lat.lua"
  "P3|/site/1m.bin|100|/tmp/lat.lua"
  "P4|/api/|100|/tmp/lat.lua"
  "P5|/api/|100|/tmp/p5.lua"
  "P6|/api/|200|/tmp/lat.lua"
  "P7|ROUTES|100|/tmp/p7.lua"
  "P8|/api/stream|100|/tmp/lat.lua"
  "P9|/site/64k.txt|100|/tmp/p9.lua"
)

for s in "${SERVERS[@]}"; do
  docker exec nodelay-wrk wrk -t2 -c50 -d 2s "${URL[$s]}/site/1k.bin" >/dev/null 2>&1 || true
done
docker exec nodelay-wrk wrk -t2 -c50 -d 3s -s /tmp/p9.lua "${URL[exyonq]}/site/64k.txt" >/dev/null 2>&1 || true

for spec in "${SCENARIOS[@]}"; do
  IFS='|' read -r sc path conns script <<<"$spec"
  for r in 1 2 3; do
    for s in "${SERVERS[@]}"; do
      tag="${s}-${sc}-r${r}"
      if [[ "$path" == "ROUTES" ]]; then
        url="${URL[$s]}"
      else
        url="${URL[$s]}${path}"
      fi
      echo "MEASURE $tag" | tee -a "$EV/summary.txt"
      measure "$tag" "${CTR[$s]}" "$url" "$conns" "$script"
      awk -v t="$tag" '/^rps /{printf "%s %s\n", t, $2}' "$EV/${tag}.wrk" | tee -a "$EV/summary.txt" || true
    done
  done
done

python3 - <<'PY' | tee "$EV/SCOREBOARD.txt"
import os, re, statistics, pathlib
ev = pathlib.Path(os.environ["EV"])
servers = ["exyonq", "nginx", "ols", "haproxy"]
scenarios = ["P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8", "P9"]

def parse_wrk(p):
    t = p.read_text(errors="ignore")
    def f(k):
        m = re.search(rf"^{k} ([0-9.]+)", t, re.M)
        return float(m.group(1)) if m else None
    return {k: f(k) for k in ["rps", "p50_us", "p90_us", "p95_us", "p99_us", "errors", "requests"]}

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
print(f"EV={ev}")
print("wrk -t2 -d12s x3 interleaved; servers=exyonq,nginx,ols,haproxy")
print("CPU=cgroup usage_usec cores; RSS=median memory.current MB")
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
        med_p95 = statistics.median(p95) if p95 else float("nan")
        med_p99 = statistics.median(p99) if p99 else float("nan")
        med_cpu = statistics.median(cpu) if cpu else float("nan")
        med_rss = statistics.median(rss) if rss else float("nan")
        print(
            f"{sc:<4} {s:<8} {statistics.median(rps):10.0f} {med_p95:8.2f} {med_p99:8.2f} {med_cpu:7.2f} {med_rss:8.1f} {max(err) if err else 0:6.0f}"
        )
        rows.append((sc, s, statistics.median(rps)))

print()
print("Skips:")
print((ev / "skips.txt").read_text())
print("vs rivals (RPS med):")
for sc in scenarios:
    by = {s: r for (sc2, s, r) in rows if sc2 == sc}
    if "exyonq" not in by:
        continue
    ex = by["exyonq"]
    for rival in ("ols", "nginx", "haproxy"):
        if rival in by and by[rival] > 0:
            pct = (ex / by[rival] - 1) * 100
            print(f"  {sc} exyonq_vs_{rival}={pct:+.1f}%  ({ex:.0f} vs {by[rival]:.0f})")
PY

echo DONE | tee -a "$EV/summary.txt" | tee "$EV/DONE"
for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik; do
  docker unpause "$c" 2>/dev/null || true
done
echo "EV=$EV"
cat "$EV/SCOREBOARD.txt"
