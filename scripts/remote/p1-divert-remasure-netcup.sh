#!/usr/bin/env bash
# Remasure P1 (static divert) + P5 (Connection:close) + P4 (api control)
# on current Cap067 EPOLL_LISTEN=1 + divert KEEP binary (no rebuild).
# DEV_NOT_OFFICIAL=1 — not rule-104 official.
set -euo pipefail

ROOT=/root/exyonq-nodelay-ab
TS=$(date -u +%Y%m%dT%H%M%SZ)
EV="$ROOT/.exyonq-local-evidence/p1-divert-remasure-$TS"
mkdir -p "$EV"
echo "$EV" > /tmp/p1-divert-ev.path
export EV

{
  echo "DEV_NOT_OFFICIAL=1"
  echo "purpose=P1 divert recovery verify + P5/P4 protectors (post AE-gate/wire-cheap/P5-handoff)"
  echo "prior_divert_KEEP=p1-divert-ab-20260920T151741Z P1 med~225k (no-divert~140k)"
  echo "prior_p5_handoff=p5-handoff-remasure-20260923T150839Z B med~43k"
  echo "gate_P1=median>=180k (divert alive; fail if ~140k Hyper-fall)"
  echo "gate_P5=median>=15k"
  echo "ts_utc=$TS"
  uptime
} | tee "$EV/meta.txt"

for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik nodelay-haproxy nodelay-nginx nodelay-ols; do
  docker pause "$c" 2>/dev/null || true
done
docker start nodelay-wrk 2>/dev/null || true

# Expect modules+EPOLL already up from prior remasure; health-check only.
health() {
  for i in 1 2 3 4 5 6 7 8 9 10; do
    if docker exec nodelayab-exyonq-1 curl -sf -m 2 http://127.0.0.1:8080/api/ >/dev/null \
      && docker exec nodelayab-exyonq-1 curl -sf -m 2 -o /dev/null http://127.0.0.1:8080/site/1k.bin; then
      return 0
    fi
    sleep 1
  done
  return 1
}
health || { echo "health fail" | tee -a "$EV/meta.txt"; exit 1; }

docker exec nodelayab-exyonq-1 printenv | grep -E 'EPOLL|WORKER|ACCEPT|CONFIG' | sort | tee "$EV/env.txt"
docker exec nodelayab-exyonq-1 sha256sum /usr/local/bin/exyonq | tee "$EV/sha.txt"
docker logs nodelayab-exyonq-1 2>&1 | grep -E 'epoll_keepalive|keepalive pool geometry|listening' | tail -5 | tee -a "$EV/meta.txt"

# Threads before
PID=$(docker exec nodelayab-exyonq-1 sh -c 'pidof exyonq | awk "{print \$1}"')
docker exec nodelayab-exyonq-1 sh -c "ls /proc/$PID/task | while read t; do cat /proc/$PID/task/\$t/comm; done" \
  | sort | uniq -c | tee "$EV/threads-before.txt"

docker exec -i nodelay-wrk sh -c 'cat >/tmp/lat.lua' <<'LUA'
done = function(summary, latency, requests)
  io.write(string.format("rps %.2f\n", summary.requests / (summary.duration / 1e6)))
  io.write(string.format("p50 %.3f\n", latency:percentile(50) / 1000))
  io.write(string.format("p99 %.3f\n", latency:percentile(99) / 1000))
end
LUA

docker exec -i nodelay-wrk sh -c 'cat >/tmp/p5.lua' <<'LUA'
wrk.method = "GET"
wrk.path = "/api/"
wrk.headers["Connection"] = "close"
done = function(summary, latency, requests)
  io.write(string.format("rps %.2f\n", summary.requests / (summary.duration / 1e6)))
  io.write(string.format("p50 %.3f\n", latency:percentile(50) / 1000))
  io.write(string.format("p99 %.3f\n", latency:percentile(99) / 1000))
end
LUA

# Warm
docker exec nodelay-wrk wrk -t2 -c50 -d 3s -s /tmp/lat.lua http://exyonq:8080/site/1k.bin >/dev/null 2>&1 || true
docker exec nodelay-wrk wrk -t2 -c50 -d 3s -s /tmp/lat.lua http://exyonq:8080/api/ >/dev/null 2>&1 || true
sleep 1

: >"$EV/summary.txt"

run3() {
  local sc="$1" url="$2" script="$3"
  for r in 1 2 3; do
    local tag="${sc}-r${r}"
    echo "--- $tag ---" | tee -a "$EV/meta.txt"
    docker exec nodelay-wrk wrk -t2 -c100 -d12s -s "$script" "$url" | tee "$EV/${tag}.wrk"
    awk -v t="$tag" '/^Requests\/sec:/{printf "%s %s\n", t, $2}' "$EV/${tag}.wrk" | tee -a "$EV/summary.txt"
    sleep 2
  done
}

# P1 first (divert primary), then P5 close, then P4 control
run3 P1 http://exyonq:8080/site/1k.bin /tmp/lat.lua
run3 P5 http://exyonq:8080/api/ /tmp/p5.lua
run3 P4 http://exyonq:8080/api/ /tmp/lat.lua

# Threads after P1 load path
docker exec nodelayab-exyonq-1 sh -c "ls /proc/$PID/task | while read t; do cat /proc/$PID/task/\$t/comm; done" \
  | sort | uniq -c | tee "$EV/threads-after.txt"

python3 - <<'PY' | tee -a "$EV/summary.txt" | tee "$EV/verdict.txt"
import re, statistics, pathlib
ev = pathlib.Path(open("/tmp/p1-divert-ev.path").read().strip())

def runs(sc):
    vals = []
    for p in sorted(ev.glob(f"{sc}-r*.wrk")):
        m = re.search(r"Requests/sec:\s+([0-9.]+)", p.read_text())
        if m:
            vals.append(float(m.group(1)))
    return vals

def med(v):
    return statistics.median(v) if v else float("nan")

p1, p5, p4 = runs("P1"), runs("P5"), runs("P4")
print(f"P1: med={med(p1):.2f} runs={p1}")
print(f"P5: med={med(p5):.2f} runs={p5}")
print(f"P4: med={med(p4):.2f} runs={p4}")
print("prior_divert_P1_med~225213 prior_no_divert~140000")
print("prior_p5_handoff_B_med~43126")

ok_p1 = med(p1) >= 180000
ok_p5 = med(p5) >= 15000
gate = "PASS" if (ok_p1 and ok_p5) else "FAIL"
print(f"GATE={gate} (P1>={180000} divert-alive, P5>={15000})")
if not ok_p1:
    print("NOTE: P1 below divert floor — Hyper-fall or divert pool dead")
if med(p1) >= 200000:
    print("NOTE: P1 in divert KEEP band (~225k)")
elif med(p1) >= 180000:
    print("NOTE: P1 above floor but below historical KEEP peak — investigate")
PY

echo DONE | tee "$EV/DONE"
echo "EV=$EV"
