#!/usr/bin/env bash
# Cap067 EPOLL + modules.metrics: prove GET /metrics completes (no hang).
# DEV_NOT_OFFICIAL=1 — causal fix verify only.
set -euo pipefail

ROOT=/root/exyonq-nodelay-ab
COMPOSE="$ROOT/.exyonq-local-evidence/tcp-nodelay-skb-20260919T123034Z/compose.yml"
TS=$(date -u +%Y%m%dT%H%M%SZ)
EV="$ROOT/.exyonq-local-evidence/cap067-metrics-hang-fix-$TS"
mkdir -p "$EV"
echo "$EV" > /tmp/cap067-metrics-hang-ev.path

{
  echo "DEV_NOT_OFFICIAL=1"
  echo "purpose=Cap067 EPOLL ON + bench-modules.toml: /metrics must 200 with bearer (no hang)"
  echo "ts_utc=$TS"
  uptime
} | tee "$EV/meta.txt"

for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik; do
  docker pause "$c" 2>/dev/null || true
done

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

# Rebuild image from synced tree (Cap067 metrics Hyper handoff).
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:${PATH}"
if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck source=/dev/null
  . "$HOME/.cargo/env"
fi

echo "=== docker rebuild exyonq (Cap067 metrics fix) ===" | tee -a "$EV/meta.txt"
docker compose -f "$COMPOSE" -f "$EV/overlay-modules-epoll.yml" -p nodelayab build exyonq 2>&1 | tee "$EV/build.log" | tail -40
docker compose -f "$COMPOSE" -f "$EV/overlay-modules-epoll.yml" -p nodelayab up -d --no-deps --force-recreate exyonq

health(){ for i in $(seq 1 90); do docker exec nodelayab-exyonq-1 curl -sf http://127.0.0.1:8080/health >/dev/null && return 0; sleep 1; done; return 1; }
health

docker exec nodelayab-exyonq-1 printenv | grep -E 'CONFIG|EPOLL|ENCODING' | tee "$EV/env.txt"
docker exec nodelayab-exyonq-1 sha256sum /usr/local/bin/exyonq /bench/bench-modules.toml | tee "$EV/sha.txt"

# Causal probes from inside exyonq (same netns as Cap067 listen).
# Prefer curl — nodelay-wrk may be stopped between suites.
set +e
docker exec nodelayab-exyonq-1 sh -c \
  'curl -sS -m 5 -D - -o /tmp/met -H "Authorization: Bearer bench-p12-metrics-token" http://127.0.0.1:8080/metrics; echo EXIT=$?; echo body=$(wc -c </tmp/met 2>/dev/null || echo 0); head -8 /tmp/met' \
  | tee "$EV/causal-metrics.txt"
docker exec nodelayab-exyonq-1 sh -c \
  'curl -sS -m 5 -D - -o /tmp/api http://127.0.0.1:8080/api/; echo EXIT=$?; echo body=$(wc -c </tmp/api 2>/dev/null || echo 0)' \
  | tee "$EV/causal-api.txt"
docker exec nodelayab-exyonq-1 sh -c \
  'curl -sS -m 5 -D - -o /tmp/hl http://127.0.0.1:8080/health; echo EXIT=$?' \
  | tee "$EV/causal-health.txt"
set -e

PASS=1
if ! grep -qE 'HTTP/1.1 200|HTTP/1.0 200' "$EV/causal-metrics.txt"; then
  echo "FAIL: /metrics not 200 under Cap067 EPOLL+modules" | tee -a "$EV/meta.txt"
  PASS=0
fi
if ! grep -qE 'body=[1-9]' "$EV/causal-metrics.txt"; then
  echo "FAIL: /metrics empty" | tee -a "$EV/meta.txt"
  PASS=0
fi
if grep -qE 'EXIT=124|timed out|Connection timed out' "$EV/causal-metrics.txt"; then
  echo "FAIL: /metrics hang/timeout under Cap067 EPOLL" | tee -a "$EV/meta.txt"
  PASS=0
fi
if ! grep -qE 'HTTP/1.1 200|HTTP/1.0 200' "$EV/causal-api.txt"; then
  echo "FAIL: /api not 200" | tee -a "$EV/meta.txt"
  PASS=0
fi
if ! grep -qE 'HTTP/1.1 200|HTTP/1.0 200' "$EV/causal-health.txt"; then
  echo "FAIL: /health not 200" | tee -a "$EV/meta.txt"
  PASS=0
fi

if [[ "$PASS" -eq 1 ]]; then
  echo "VERDICT=PASS Cap067 EPOLL /metrics no-hang" | tee -a "$EV/meta.txt" | tee "$EV/verdict.txt"
  touch "$EV/DONE"
  exit 0
fi
echo "VERDICT=FAIL" | tee -a "$EV/meta.txt" | tee "$EV/verdict.txt"
exit 1
