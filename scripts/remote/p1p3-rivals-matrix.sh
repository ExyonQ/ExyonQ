#!/usr/bin/env bash
# Development P1–P3 matrix. Not an official bench (one 15s window, not 30s×reps).
set -euo pipefail
ROOT=/root/exyonq-nodelay-ab
EV=$ROOT/.exyonq-local-evidence/p1p3-rivals-$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$EV"
echo "$EV" > /tmp/p1p3-ev.path

# OLS may have exited; the static config is the same one already verified.
if ! docker inspect nodelay-ols >/dev/null 2>&1 || ! docker inspect -f '{{.State.Running}}' nodelay-ols | grep -q true; then
  docker rm -f nodelay-ols >/dev/null 2>&1 || true
  docker run -d --name nodelay-ols --network nodelayab_default --cpuset-cpus 0-7 \
    -v "$ROOT/benchmarks/configs/openlitespeed/httpd_config.conf:/usr/local/lsws/conf/httpd_config.conf:ro" \
    -v "$ROOT/benchmarks/configs/openlitespeed/vhosts/bench/vhconf.conf:/usr/local/lsws/conf/vhosts/bench/vhconf.conf:ro" \
    -v "$ROOT/benchmarks/scenarios/payloads/www:/bench/www:ro" \
    -v "$ROOT/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro" \
    litespeedtech/openlitespeed:1.9.0-lsphp83 >/dev/null
  sleep 2
fi

declare -A URL CTR
URL[exyonq]=http://exyonq:8080
CTR[exyonq]=nodelayab-exyonq-1
URL[nginx]=http://nginx:8080
CTR[nginx]=nodelayab-nginx-1
URL[caddy]=http://nodelay-caddy:8080
CTR[caddy]=nodelay-caddy
URL[apache]=http://nodelay-apache:8080
CTR[apache]=nodelay-apache
URL[ols]=http://nodelay-ols:8088
CTR[ols]=nodelay-ols
URL[haproxy]=http://nodelay-haproxy:8080
CTR[haproxy]=nodelay-haproxy
URL[envoy]=http://nodelay-envoy:8080
CTR[envoy]=nodelay-envoy
URL[traefik]=http://nodelay-traefik:8080
CTR[traefik]=nodelay-traefik

echo "server scenario bytes" >"$EV/identity.tsv"
for name in exyonq nginx caddy apache ols haproxy envoy traefik; do
  for spec in "P1|/site/1k.bin|1024" "P2|/site/64k.bin|65536" "P3|/site/1m.bin|1048576"; do
    sc=${spec%%|*}; rest=${spec#*|}; path=${rest%%|*}; want=${rest##*|}
    got=$(docker exec nodelay-wrk wget -q -O /tmp/body.bin "${URL[$name]}$path" && docker exec nodelay-wrk wc -c </tmp/body.bin || echo FAIL)
    echo "$name $sc $got want=$want" | tee -a "$EV/identity.tsv"
  done
done

measure() {
  local tag=$1 cname=$2 url=$3
  docker exec "$cname" cat /sys/fs/cgroup/cpu.stat >"$EV/${tag}.cpu0" || true
  local t0=$(date +%s%N)
  (
    for _ in 1 2 3 4 5; do
      docker exec "$cname" cat /sys/fs/cgroup/memory.current
      sleep 2
    done
  ) >"$EV/${tag}.mem" &
  local mp=$!
  docker exec nodelay-wrk wrk -t2 -c100 -d 15s -s /tmp/lat.lua "$url" >"$EV/${tag}.wrk" || true
  wait "$mp" || true
  local t1=$(date +%s%N)
  echo $((t1 - t0)) >"$EV/${tag}.wallns"
  docker exec "$cname" cat /sys/fs/cgroup/cpu.stat >"$EV/${tag}.cpu1" || true
}

for name in exyonq nginx caddy apache ols haproxy envoy traefik; do
  docker exec nodelay-wrk wrk -t2 -c100 -d 3s "${URL[$name]}/site/1k.bin" >/dev/null || true
  for spec in "P1|/site/1k.bin" "P2|/site/64k.bin" "P3|/site/1m.bin"; do
    sc=${spec%%|*}; path=${spec#*|}
    echo "MEASURE $name $sc"
    measure "${name}-${sc}" "${CTR[$name]}" "${URL[$name]}${path}"
  done
done
echo DONE >"$EV/DONE"
echo EV=$EV
