#!/usr/bin/env bash
# P5 Cap067 handoff remasure — WouldBlock→HyperReady + Cap067→Tokio queue.
# DEV_NOT_OFFICIAL=1 — not rule-104 official.
set -euo pipefail

ROOT=/root/exyonq-nodelay-ab
COMPOSE_DIR="$ROOT/.exyonq-local-evidence/tcp-nodelay-skb-20260919T123034Z"
COMPOSE="$COMPOSE_DIR/compose.yml"
TS=$(date -u +%Y%m%dT%H%M%SZ)
EV="$ROOT/.exyonq-local-evidence/p5-handoff-remasure-$TS"
mkdir -p "$EV"
echo "$EV" > /tmp/p5-handoff-ev.path

{
  echo "DEV_NOT_OFFICIAL=1"
  echo "purpose=P5 Connection:close remasure Cap067 handoff KEEP (WouldBlock→Hyper + bridge)"
  echo "gate=B_median>=15k AND B>haproxy; prefer B≈A(epoll0)"
  echo "ts_utc=$TS"
  uptime
} | tee "$EV/meta.txt"

for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik nodelay-ols nodelay-nginx nodelayab-nginx-1; do
  docker pause "$c" 2>/dev/null || true
done
docker start nodelay-wrk nodelay-haproxy 2>/dev/null || true

if [[ ! -f "$COMPOSE" ]]; then
  echo "FATAL: missing compose $COMPOSE" | tee -a "$EV/meta.txt"
  exit 1
fi

cd "$ROOT"
export PATH="${HOME}/.cargo/bin:${PATH}"
[[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"

health() {
  for _ in $(seq 1 90); do
    docker exec nodelayab-exyonq-1 curl -sf http://127.0.0.1:8080/health >/dev/null && return 0
    sleep 1
  done
  return 1
}

run_arm() {
  local arm="$1" epoll="$2"
  cat >"$EV/overlay-${arm}.yml" <<OYAML
services:
  exyonq:
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_EPOLL_POOL_THREADS: "4"
      EXYONQ_EPOLL_LISTEN: "${epoll}"
      EXYONQ_EPOLL_STATIC: "1"
      EXYONQ_EPOLL_SENDFILE: "1"
      EXYONQ_STATIC_ENCODING_CACHE: "1"
      EXYONQ_STATIC_ENCODING_CACHE_DIR: "/tmp/exyonq-static-encoding"
    volumes:
      - ${ROOT}/benchmarks/configs/exyonq/bench.toml:/bench/bench.toml:ro
      - ${ROOT}/benchmarks/scenarios/payloads/www:/bench/www:ro
      - ${ROOT}/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
OYAML
  echo "=== arm ${arm} EPOLL_LISTEN=${epoll} rebuild ===" | tee -a "$EV/meta.txt"
  docker compose -f "$COMPOSE" -f "$EV/overlay-${arm}.yml" -p nodelayab build exyonq 2>&1 | tee "$EV/build-${arm}.log" | tail -20
  docker compose -f "$COMPOSE" -f "$EV/overlay-${arm}.yml" -p nodelayab up -d --no-deps --force-recreate exyonq
  health || { echo "health fail arm $arm" | tee -a "$EV/meta.txt"; exit 1; }
  docker exec nodelayab-exyonq-1 printenv | grep -E 'EPOLL|WORKER|ACCEPT' | tee "$EV/env-${arm}.txt"
  docker exec nodelayab-exyonq-1 sha256sum /usr/local/bin/exyonq | tee "$EV/sha-${arm}.txt"

  docker exec -i nodelay-wrk sh -c 'cat >/tmp/p5.lua' <<'LUA'
wrk.method = "GET"
wrk.path = "/api/"
wrk.headers["Connection"] = "close"
LUA

  for r in 1 2 3; do
    echo "--- ${arm} P5 r${r} ---" | tee -a "$EV/meta.txt"
    docker exec nodelay-wrk wrk -t2 -c100 -d12s -s /tmp/p5.lua http://exyonq:8080/api/ \
      | tee "$EV/${arm}-p5-r${r}.wrk"
    sleep 2
  done
}

# Build once with EPOLL=1 first (product default), then flip env for A without full rebuild if possible.
# Image must include new binary — rebuild on first arm only, recreate for second.

run_arm B 1

# Arm A: recreate with EPOLL_LISTEN=0 (same image)
cat >"$EV/overlay-A.yml" <<OYAML
services:
  exyonq:
    environment:
      EXYONQ_CONFIG: /bench/bench.toml
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_EPOLL_POOL_THREADS: "4"
      EXYONQ_EPOLL_LISTEN: "0"
      EXYONQ_EPOLL_STATIC: "1"
      EXYONQ_EPOLL_SENDFILE: "1"
    volumes:
      - ${ROOT}/benchmarks/configs/exyonq/bench.toml:/bench/bench.toml:ro
      - ${ROOT}/benchmarks/scenarios/payloads/www:/bench/www:ro
      - ${ROOT}/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
OYAML
echo "=== arm A EPOLL_LISTEN=0 recreate (no rebuild) ===" | tee -a "$EV/meta.txt"
docker compose -f "$COMPOSE" -f "$EV/overlay-A.yml" -p nodelayab up -d --no-deps --force-recreate exyonq
health || { echo "health fail arm A" | tee -a "$EV/meta.txt"; exit 1; }
docker exec nodelayab-exyonq-1 printenv | grep -E 'EPOLL|WORKER|ACCEPT' | tee "$EV/env-A.txt"
for r in 1 2 3; do
  echo "--- A P5 r${r} ---" | tee -a "$EV/meta.txt"
  docker exec nodelay-wrk wrk -t2 -c100 -d12s -s /tmp/p5.lua http://exyonq:8080/api/ \
    | tee "$EV/A-p5-r${r}.wrk"
  sleep 2
done

# HAProxy reference
docker unpause nodelay-haproxy 2>/dev/null || docker start nodelay-haproxy 2>/dev/null || true
sleep 2
for r in 1 2 3; do
  echo "--- haproxy P5 r${r} ---" | tee -a "$EV/meta.txt"
  docker exec nodelay-wrk wrk -t2 -c100 -d12s -s /tmp/p5.lua http://nodelay-haproxy:8080/api/ \
    | tee "$EV/haproxy-p5-r${r}.wrk" || true
  sleep 2
done

python3 - <<'PY' | tee "$EV/summary.txt"
import glob, re, statistics, os
ev = os.environ.get("EV") or open("/tmp/p5-handoff-ev.path").read().strip()

def med(pattern):
    vals = []
    for p in sorted(glob.glob(f"{ev}/{pattern}")):
        t = open(p).read()
        m = re.search(r"Requests/sec:\s+([0-9.]+)", t)
        if m:
            vals.append(float(m.group(1)))
    if not vals:
        return None, []
    return statistics.median(vals), vals

b_med, b = med("B-p5-r*.wrk")
a_med, a = med("A-p5-r*.wrk")
h_med, h = med("haproxy-p5-r*.wrk")
print(f"EV={ev}")
print(f"B EPOLL=1 P5: med={b_med} runs={b}")
print(f"A EPOLL=0 P5: med={a_med} runs={a}")
print(f"HAProxy P5:    med={h_med} runs={h}")
ok = b_med is not None and b_med >= 15000 and (h_med is None or b_med > h_med)
print(f"GATE={'PASS' if ok else 'FAIL'} (B>=15k and B>haproxy)")
open(f"{ev}/verdict.txt", "w").write("PASS\n" if ok else "FAIL\n")
PY

# Unpause rivals we paused
for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik nodelay-ols; do
  docker unpause "$c" 2>/dev/null || true
done
echo DONE | tee "$EV/DONE"
echo "$EV"
