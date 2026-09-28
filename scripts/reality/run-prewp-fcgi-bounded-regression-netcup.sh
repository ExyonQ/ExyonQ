#!/usr/bin/env bash
# V044 pre-WordPress FastCGI bounded regression (Netcup amd64).
# NOT full P6D stress. KEEP_CONN + gen reload + nonempty script_suffix reject.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6-fastcgi-pre-wordpress-blocker-repair-attribution-close}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
WORKDIR="${WORKDIR:-/tmp/exyonq-prewp-$RUN_ID}"
LISTEN="${LISTEN:-127.0.0.1:18093}"
FPM_PORT="${FPM_PORT:-19093}"
POOL_MAX="${POOL_MAX:-1}"
FPM_CHILDREN="${FPM_CHILDREN:-4}"
IDLE_MS="${IDLE_MS:-60000}"
REQS="${REQS:-200}"
DOCROOT="$WORKDIR/www"
GEN_DIR="$WORKDIR/gen"
FPM_CONF="$WORKDIR/php-fpm.conf"
FPM_POOL="$WORKDIR/pool.conf"
ROUTES="$WORKDIR/routes.txt"
STAGE_DIR="$EVIDENCE_ROOT/$RUN_ID"

mkdir -p "$DOCROOT" "$GEN_DIR" "$STAGE_DIR/STAGES" "$STAGE_DIR/ORACLE_SYN" "$STAGE_DIR/FPM"
cd "$ROOT"

FPM_USER="${FPM_USER:-}"
FPM_GROUP="${FPM_GROUP:-}"
if [[ -z "$FPM_USER" ]]; then
  if [[ "$(id -u)" -eq 0 ]]; then
    for cand in www-data nginx nobody; do
      if id -u "$cand" >/dev/null 2>&1; then
        FPM_USER=$cand
        FPM_GROUP=$(id -gn "$cand")
        break
      fi
    done
  else
    FPM_USER=$(id -un)
    FPM_GROUP=$(id -gn)
  fi
fi

cat >"$DOCROOT/marker.php" <<'PHP'
<?php
header('Content-Type: text/plain; charset=utf-8');
echo "PREWP_DYN nonce=" . bin2hex(random_bytes(8)) . "\n";
PHP
chmod 644 "$DOCROOT/marker.php"
[[ "$(id -u)" -eq 0 ]] && chown -R "$FPM_USER:$FPM_GROUP" "$DOCROOT" || true

write_fpm_pool() {
  cat >"$FPM_POOL" <<EOF
[www]
user = ${FPM_USER}
group = ${FPM_GROUP}
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = ${FPM_CHILDREN}
security.limit_extensions = .php
EOF
  cat >"$FPM_CONF" <<EOF
[global]
pid = $WORKDIR/php-fpm.pid
error_log = $WORKDIR/php-fpm.log
daemonize = no
include = $FPM_POOL
EOF
}

write_routes() {
  local maxc="$1" idle="$2"
  cat >"$ROUTES" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|${maxc}|${idle}|2000|30000|30000|60000
|/marker.php|fcgi:1
EOF
}

PHP_FPM_BIN=""
for c in /usr/sbin/php-fpm8.3 /usr/sbin/php-fpm php-fpm8.3 php-fpm; do
  if [[ -x "$c" ]] || command -v "$c" >/dev/null 2>&1; then
    PHP_FPM_BIN=$(command -v "$c" 2>/dev/null || echo "$c")
    break
  fi
done
[[ -n "$PHP_FPM_BIN" ]] || { echo "FAIL: php-fpm"; exit 2; }

FPM_PID="" DP_PID="" TCPDUMP_PID=""
cleanup() {
  [[ -n "${TCPDUMP_PID:-}" ]] && kill "$TCPDUMP_PID" 2>/dev/null || true
  [[ -n "${DP_PID:-}" ]] && kill "$DP_PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL: binaries"; exit 3; }
sha256sum "$BIN" | tee "$STAGE_DIR/BINARY_SHA256.txt"

write_fpm_pool
"$PHP_FPM_BIN" -y "$FPM_CONF" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
for _ in $(seq 1 100); do
  (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1 && break
  sleep 0.1
done

# A2 reject
cat >"$WORKDIR/bad.routes" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|1|60000|2000|30000|30000|60000|.php
|/marker.php|fcgi:1
EOF
set +e
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/bad.routes" --generation-id 99 >"$STAGE_DIR/FPM/suffix.out" 2>"$STAGE_DIR/FPM/suffix.err"
SUF_RC=$?
set -e
[[ "$SUF_RC" -ne 0 ]] || { echo "FAIL: nonempty script_suffix accepted"; exit 2; }
echo "SUFFIX_REJECT_PASS=1 rc=$SUF_RC" | tee "$STAGE_DIR/STAGES/A2.txt"

write_routes "$POOL_MAX" "$IDLE_MS"
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 1 | tee "$STAGE_DIR/FPM/publish.txt"

"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards 1 --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
for _ in $(seq 1 150); do
  if kill -0 "$DP_PID" 2>/dev/null && grep -q READY "$WORKDIR/dp.err" 2>/dev/null; then break; fi
  if ! kill -0 "$DP_PID" 2>/dev/null; then
    echo "FAIL: dataplane died"; cat "$WORKDIR/dp.err"; exit 2
  fi
  sleep 0.1
done

# Capture SYNs from first request (no pre-warm) for KEEP_CONN oracle
PCAP="$STAGE_DIR/ORACLE_SYN/keep.pcap"
tcpdump -i lo -nn -w "$PCAP" "tcp and dst port $FPM_PORT and tcp[tcpflags] & tcp-syn != 0" >/dev/null 2>&1 &
TCPDUMP_PID=$!
sleep 0.3
OK=0
FAIL=0
for i in $(seq 1 "$REQS"); do
  if curl -fsS -o /dev/null --http1.1 "http://$LISTEN/marker.php?rid=$i"; then
    OK=$((OK + 1))
  else
    FAIL=$((FAIL + 1))
  fi
done
sleep 0.3
kill "$TCPDUMP_PID" 2>/dev/null || true
wait "$TCPDUMP_PID" 2>/dev/null || true
TCPDUMP_PID=""
SYN=$(tcpdump -nn -r "$PCAP" 2>/dev/null | wc -l | tr -d ' ')
echo "KEEP_CONN OK=$OK FAIL=$FAIL REQS=$REQS SYN=$SYN" | tee "$STAGE_DIR/STAGES/KEEP_CONN.txt"
# Independent oracle: many requests, few backend SYNs (reuse). SYN>=1 required for cold start.
[[ "$OK" -eq "$REQS" && "$FAIL" -eq 0 && "$SYN" -ge 1 && "$SYN" -lt "$REQS" ]] || {
  echo "FAIL: KEEP_CONN"
  exit 2
}

# Generation / TTL reload via publish into gen-dir (hot reload watches gen-dir)
write_routes "$POOL_MAX" 2000
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 2 | tee -a "$STAGE_DIR/FPM/publish.txt"
sleep 0.5
curl -fsS -o "$WORKDIR/body_ttl" "http://$LISTEN/marker.php?rid=ttl"
grep -q PREWP_DYN "$WORKDIR/body_ttl"
# Honesty: survival after publish only — not ADR-040 policy oracle (P6PREWP-LA-001)
echo "TTL_RELOAD=SURVIVAL_AFTER_PUBLISH" | tee "$STAGE_DIR/STAGES/TTL.txt"

write_routes 2 "$IDLE_MS"
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 3 | tee -a "$STAGE_DIR/FPM/publish.txt"
sleep 0.5
curl -fsS -o "$WORKDIR/body_max" "http://$LISTEN/marker.php?rid=max"
grep -q PREWP_DYN "$WORKDIR/body_max"
echo "MAX_CONNECTIONS_RELOAD=SURVIVAL_AFTER_PUBLISH" | tee "$STAGE_DIR/STAGES/MAX.txt"
echo "GENERATION_RELOAD=SURVIVAL_AFTER_PUBLISH" | tee "$STAGE_DIR/STAGES/GEN.txt"

{
  echo "PREWP_REGRESSION_PASS=1"
  echo "KEEP_CONN_REQUESTS=$OK"
  echo "KEEP_CONN_BACKEND_ACCEPTS=$SYN"
  echo "BINARY_SHA256=$(awk '{print $1}' "$STAGE_DIR/BINARY_SHA256.txt")"
  echo "SUFFIX_REJECT_PASS=1"
  echo "TTL_RELOAD=SURVIVAL_AFTER_PUBLISH"
  echo "MAX_CONNECTIONS_RELOAD=SURVIVAL_AFTER_PUBLISH"
  echo "GENERATION_RELOAD=SURVIVAL_AFTER_PUBLISH"
  echo "POLICY_RELOAD_ORACLE=NOT_MEASURED_THIS_BOUNDED_HARNESS"
} | tee "$STAGE_DIR/SUMMARY.txt"
