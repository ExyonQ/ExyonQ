#!/usr/bin/env bash
# P11 TLS+H2 DEV lab on Netcup nodelayab — unblock SKIP_NO_TLS_ON_NODELAYAB.
# Measurement: DEV_NOT_OFFICIAL / NOT_FOR_RANKING / PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN
# Kill switch: restore modules cleartext overlay at end.
set -euo pipefail

ROOT=/root/exyonq-nodelay-ab
COMPOSE="$ROOT/.exyonq-local-evidence/tcp-nodelay-skb-20260919T123034Z/compose.yml"
REWRK="${REWRK:-$HOME/.cargo/bin/rewrk}"
TS=$(date -u +%Y%m%dT%H%M%SZ)
EV="$ROOT/.exyonq-local-evidence/p11-tls-lab-$TS"
mkdir -p "$EV/tls"
echo "$EV" > /tmp/p11-tls-lab-ev.path
export EV

{
  echo "DEV_NOT_OFFICIAL=1"
  echo "measurement_class=DEV_LAB"
  echo "NOT_FOR_RANKING=1"
  echo "NOT_FOR_RELEASE=1"
  echo "PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN"
  echo "purpose=P11 TLS+H2 lab unblock — ExyonQ + nginx on :8443 ALPN h2"
  echo "path=/site/1k.bin (scenarios.toml p11)"
  echo "loadgen=rewrk --http2"
  echo "ts_utc=$TS"
  uptime
} | tee "$EV/meta.txt"

command -v "$REWRK" >/dev/null || { echo "rewrk missing at $REWRK" | tee -a "$EV/meta.txt"; exit 1; }
command -v openssl >/dev/null || { echo "openssl required" | tee -a "$EV/meta.txt"; exit 1; }

# Lab SANs: host rewrk via published 127.0.0.1 + in-network names
CFG="$EV/tls/openssl.cnf"
cat >"$CFG" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = v3_req
prompt = no
[dn]
CN = localhost
[v3_req]
subjectAltName = @alt_names
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
[alt_names]
DNS.1 = localhost
DNS.2 = exyonq
DNS.3 = nginx
DNS.4 = nodelayab-exyonq-1
DNS.5 = nodelayab-nginx-1
IP.1 = 127.0.0.1
IP.2 = ::1
EOF
openssl req -x509 -newkey rsa:2048 -nodes \
  -keyout "$EV/tls/key.pem" -out "$EV/tls/cert.pem" \
  -days 1 -config "$CFG" -extensions v3_req >/dev/null 2>&1
chmod 600 "$EV/tls/key.pem"
chmod 644 "$EV/tls/cert.pem"
rm -f "$CFG"
echo "tls_material=$EV/tls" | tee -a "$EV/meta.txt"

# Pause noisy rivals (keep nginx for compare)
for c in nodelay-caddy nodelay-apache nodelay-envoy nodelay-traefik nodelay-haproxy nodelay-ols; do
  docker pause "$c" 2>/dev/null || true
done
docker unpause nodelayab-nginx-1 2>/dev/null || docker start nodelayab-nginx-1 2>/dev/null || true
docker start nodelay-wrk 2>/dev/null || true

# Product serve binds primary listen only — use stock bench-tls.toml (:8443 TLS).
# Dual-listen lab toml is documentation-only until multi-listen is product-supported.
cat >"$EV/overlay-p11.yml" <<OYAML
services:
  exyonq:
    environment:
      EXYONQ_CONFIG: /bench/bench-tls.toml
      EXYONQ_WORKER_THREADS: "4"
      EXYONQ_ACCEPT_WORKERS: "4"
      EXYONQ_EPOLL_POOL_THREADS: "4"
      # Cap067 EPOLL listen is cleartext-first; TLS+h2 uses Hyper (do not claim wire-cheap).
      EXYONQ_EPOLL_LISTEN: "0"
      EXYONQ_EPOLL_STATIC: "0"
      EXYONQ_EPOLL_SENDFILE: "0"
    ports:
      - "18443:8443"
    volumes:
      - ${ROOT}/benchmarks/configs/exyonq/bench-tls.toml:/bench/bench-tls.toml:ro
      - ${ROOT}/benchmarks/configs/exyonq/bench.toml:/bench/bench.toml:ro
      - ${ROOT}/benchmarks/scenarios/payloads/www:/bench/www:ro
      - ${ROOT}/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
      - ${EV}/tls:/bench/tls:ro
    healthcheck:
      test: ["CMD", "curl", "-sfk", "https://127.0.0.1:8443/site/1k.bin"]
      interval: 5s
      timeout: 3s
      retries: 12
      start_period: 8s
  nginx:
    volumes:
      - ${ROOT}/benchmarks/configs/nginx/nginx-tls-p11.conf:/etc/nginx/nginx.conf:ro
      - ${ROOT}/benchmarks/scenarios/payloads/www:/bench/www:ro
      - ${ROOT}/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
      - ${EV}/tls:/bench/tls:ro
    ports:
      - "18481:8443"
OYAML

echo "=== bring up TLS lab overlay ===" | tee -a "$EV/meta.txt"
docker compose -f "$COMPOSE" -f "$EV/overlay-p11.yml" -p nodelayab up -d --no-deps --force-recreate exyonq nginx

smoke_ok=0
for i in $(seq 1 30); do
  if curl -sfk -m 2 -o /dev/null https://127.0.0.1:18443/site/1k.bin \
    && curl -sfk -m 2 -o /dev/null https://127.0.0.1:18481/site/1k.bin; then
    smoke_ok=1
    break
  fi
  sleep 1
done
if [[ "$smoke_ok" != 1 ]]; then
  echo "SMOKE_FAIL cleartext/tls fetch" | tee -a "$EV/meta.txt"
  docker logs nodelayab-exyonq-1 2>&1 | tail -40 | tee "$EV/exyonq-boot.log"
  docker logs nodelayab-nginx-1 2>&1 | tail -40 | tee "$EV/nginx-boot.log"
  exit 1
fi
echo "SMOKE_HTTP_OK" | tee -a "$EV/meta.txt"

# ALPN h2 probe
alpn_exy=$(echo | openssl s_client -connect 127.0.0.1:18443 -alpn h2 -servername localhost 2>/dev/null \
  | awk '/ALPN protocol:/{print $3; exit}')
alpn_ngx=$(echo | openssl s_client -connect 127.0.0.1:18481 -alpn h2 -servername localhost 2>/dev/null \
  | awk '/ALPN protocol:/{print $3; exit}')
echo "alpn_exyonq=${alpn_exy:-none}" | tee -a "$EV/meta.txt" | tee "$EV/alpn.txt"
echo "alpn_nginx=${alpn_ngx:-none}" | tee -a "$EV/meta.txt" | tee -a "$EV/alpn.txt"
if [[ "${alpn_exy:-}" != "h2" || "${alpn_ngx:-}" != "h2" ]]; then
  echo "ALPN_FAIL need h2 on both" | tee -a "$EV/meta.txt"
  exit 1
fi
echo "SMOKE_ALPN_H2_OK" | tee -a "$EV/meta.txt"

docker exec nodelayab-exyonq-1 printenv | grep -E 'CONFIG|EPOLL|WORKER|ACCEPT' | sort | tee "$EV/env-exyonq.txt"
docker exec nodelayab-exyonq-1 sha256sum /usr/local/bin/exyonq | tee "$EV/sha.txt"

: >"$EV/summary.txt"
run3() {
  local tag="$1" url="$2"
  for r in 1 2 3; do
    echo "--- ${tag}-r${r} ---" | tee -a "$EV/meta.txt"
    # 12s DEV cell (not official 30s)
    "$REWRK" --http2 -t2 -c100 -d 12s -h "$url" 2>&1 | tee "$EV/${tag}-r${r}.rewrk"
    # rewrk 0.3: "Req/Sec: 65576.68" (never use Total request count)
    python3 - "$EV/${tag}-r${r}.rewrk" "${tag}-r${r}" <<'PY' | tee -a "$EV/summary.txt"
import re, sys
t = open(sys.argv[1]).read()
tag = sys.argv[2]
m = re.search(r"Req/Sec:\s*([0-9.]+)", t) or re.search(r"Requests/sec:\s*([0-9.]+)", t)
if not m:
    sys.exit(f"parse fail {tag}")
print(f"{tag} {m.group(1)}")
PY
    sleep 2
  done
}

run3 exyonq https://127.0.0.1:18443/site/1k.bin
run3 nginx https://127.0.0.1:18481/site/1k.bin

python3 - <<'PY' | tee -a "$EV/summary.txt" | tee "$EV/verdict.txt"
import re, statistics, pathlib
ev = pathlib.Path(open("/tmp/p11-tls-lab-ev.path").read().strip())
alpn = (ev / "alpn.txt").read_text() if (ev / "alpn.txt").exists() else ""

def runs(prefix):
    vals = []
    for p in sorted(ev.glob(f"{prefix}-r*.rewrk")):
        t = p.read_text()
        m = re.search(r"Req/Sec:\s*([0-9.]+)", t) or re.search(r"Requests/sec:\s*([0-9.]+)", t)
        if m:
            vals.append(float(m.group(1)))
    return vals

def med(v):
    return statistics.median(v) if v else float("nan")

ex, ng = runs("exyonq"), runs("nginx")
print(f"exyonq P11: med={med(ex):.2f} runs={ex}")
print(f"nginx  P11: med={med(ng):.2f} runs={ng}")
print(alpn.strip())
ok_alpn = "alpn_exyonq=h2" in alpn and "alpn_nginx=h2" in alpn
ok_rps = bool(ex) and bool(ng) and med(ex) > 0 and med(ng) > 0
gate = "PASS_LAB_RUNNABLE" if (ok_alpn and ok_rps) else "FAIL"
print(f"GATE={gate}")
print("DEV_NOT_OFFICIAL=1 NOT_FOR_RANKING=1 PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN")
print("NOTE: no Cap067 wire-cheap claim on TLS; Hyper/h2 path expected")
PY

# Kill switch — restore cleartext modules stack
PREV="$ROOT/.exyonq-local-evidence/p10p12-modules-epoll-on-20260923T153150Z/overlay-modules-epoll.yml"
if [[ -f "$PREV" ]]; then
  echo "=== kill switch: restore modules cleartext ===" | tee -a "$EV/meta.txt"
  # Restore stock nginx.conf
  cat >"$EV/overlay-restore-nginx.yml" <<OYAML
services:
  nginx:
    volumes:
      - ${ROOT}/benchmarks/configs/nginx/nginx.conf:/etc/nginx/nginx.conf:ro
      - ${ROOT}/benchmarks/scenarios/payloads/www:/bench/www:ro
      - ${ROOT}/benchmarks/scenarios/payloads/health.txt:/bench/health.txt:ro
OYAML
  docker compose -f "$COMPOSE" -f "$PREV" -f "$EV/overlay-restore-nginx.yml" -p nodelayab \
    up -d --no-deps --force-recreate exyonq nginx
  sleep 2
  docker exec nodelayab-exyonq-1 printenv EXYONQ_CONFIG | tee -a "$EV/meta.txt"
fi

echo DONE | tee "$EV/DONE"
echo "EV=$EV"
