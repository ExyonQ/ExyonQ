#!/usr/bin/env bash
# V044 — CFD FastCGI max_connections config honesty (ADR-042) on Netcup amd64.
# WIP: V044_PHASE6_FASTCGI_MAX_CONNECTIONS_CONFIG_HONESTY_CLOSE
# Proves: max_conn!=1 rejected; max_conn=1 publishes; KEEP_CONN reuse; gen reload max=1.
# ZERO_FAKE / NO_SMOKE / NO concurrency manufacture.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6-fastcgi-max-connections-config-honesty-close}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
WORKDIR="${WORKDIR:-/tmp/exyonq-p6maxhonest-$RUN_ID}"
LISTEN="${LISTEN:-127.0.0.1:18094}"
FPM_PORT="${FPM_PORT:-19094}"
FPM_CHILDREN="${FPM_CHILDREN:-4}"
REQS="${REQS:-200}"
DOCROOT="$WORKDIR/www"
GEN_DIR="$WORKDIR/gen"
FPM_CONF="$WORKDIR/php-fpm.conf"
FPM_POOL="$WORKDIR/pool.conf"
ROUTES="$WORKDIR/routes.txt"
STAGE_DIR="$EVIDENCE_ROOT/$RUN_ID"

mkdir -p "$DOCROOT" "$GEN_DIR" "$STAGE_DIR"/{STAGES,ORACLE_SYN,FPM,REJECT}
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
echo "P6MAXHONEST_DYN nonce=" . bin2hex(random_bytes(8)) . "\n";
PHP
chmod 644 "$DOCROOT/marker.php"
[[ "$(id -u)" -eq 0 ]] && chown -R "$FPM_USER:$FPM_GROUP" "$DOCROOT" || true

write_fpm() {
  cat >"$FPM_POOL" <<EOF
[www]
user = ${FPM_USER}
group = ${FPM_GROUP}
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = ${FPM_CHILDREN}
clear_env = no
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

write_fpm
"$PHP_FPM_BIN" -y "$FPM_CONF" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
for _ in $(seq 1 100); do
  (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1 && break
  sleep 0.1
done

# --- Reject max_conn=6 before any good gen ---
write_routes 6 60000
set +e
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 1 \
  >"$STAGE_DIR/REJECT/max6.out" 2>"$STAGE_DIR/REJECT/max6.err"
RC6=$?
set -e
[[ "$RC6" -ne 0 ]] || { echo "FAIL: max_conn=6 accepted"; exit 2; }
grep -qi 'ADR-042\|max_conn=6\|serial' "$STAGE_DIR/REJECT/max6.err" "$STAGE_DIR/REJECT/max6.out" \
  || { echo "FAIL: reject message missing ADR-042 detail"; cat "$STAGE_DIR/REJECT/max6.err"; exit 2; }
echo "REJECT_MAX6_PASS=1 rc=$RC6" | tee "$STAGE_DIR/STAGES/REJECT_GT1.txt"

# --- Reject max_conn=0 ---
write_routes 0 60000
set +e
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 1 \
  >"$STAGE_DIR/REJECT/max0.out" 2>"$STAGE_DIR/REJECT/max0.err"
RC0=$?
set -e
[[ "$RC0" -ne 0 ]] || { echo "FAIL: max_conn=0 accepted"; exit 2; }
echo "REJECT_MAX0_PASS=1 rc=$RC0" | tee -a "$STAGE_DIR/STAGES/REJECT_GT1.txt"

# --- Accept max_conn=1 ---
write_routes 1 60000
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 1 | tee "$STAGE_DIR/FPM/publish1.txt"

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

BODY=$(curl -fsS --http1.1 "http://$LISTEN/marker.php?rid=boot")
echo "$BODY" | grep -q P6MAXHONEST_DYN
echo "DYNAMIC_PHP_PASS=1" | tee "$STAGE_DIR/STAGES/DYNAMIC.txt"

# --- Bad reload must not displace gen 1 ---
MARKER_BEFORE=$(curl -fsS --http1.1 "http://$LISTEN/marker.php?rid=prebad")
write_routes 6 60000
set +e
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 2 \
  >"$STAGE_DIR/REJECT/reload6.out" 2>"$STAGE_DIR/REJECT/reload6.err"
RC_RELOAD=$?
set -e
[[ "$RC_RELOAD" -ne 0 ]] || { echo "FAIL: reload max=6 accepted"; exit 2; }
sleep 0.3
MARKER_AFTER=$(curl -fsS --http1.1 "http://$LISTEN/marker.php?rid=postbad")
echo "$MARKER_AFTER" | grep -q P6MAXHONEST_DYN
echo "FAILED_PUBLISH_NO_EXTERNAL_EFFECT=PASS before=${#MARKER_BEFORE} after=${#MARKER_AFTER}" \
  | tee "$STAGE_DIR/STAGES/RELOAD_REJECT.txt"

# --- Valid gen 2 with max=1 ---
write_routes 1 2000
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 2 | tee "$STAGE_DIR/FPM/publish2.txt"
sleep 0.4
curl -fsS --http1.1 "http://$LISTEN/marker.php?rid=gen2" | grep -q P6MAXHONEST_DYN
echo "GEN_RELOAD_MAX1_PASS=1" | tee "$STAGE_DIR/STAGES/GEN_RELOAD.txt"

# --- KEEP_CONN (cold dataplane so SYN oracle sees first connect) ---
kill "$DP_PID" 2>/dev/null || true
wait "$DP_PID" 2>/dev/null || true
DP_PID=""
write_routes 1 60000
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 3 | tee "$STAGE_DIR/FPM/publish3.txt"
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
PCAP="$STAGE_DIR/ORACLE_SYN/keep.pcap"
tcpdump -i lo -nn -w "$PCAP" "tcp and dst port $FPM_PORT and tcp[tcpflags] & tcp-syn != 0" >/dev/null 2>&1 &
TCPDUMP_PID=$!
sleep 0.3
OK=0
FAIL=0
for i in $(seq 1 "$REQS"); do
  if curl -fsS -o /dev/null --http1.1 -H 'Connection: close' "http://$LISTEN/marker.php?rid=k$i"; then
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
KEEP=FAIL
[[ "$OK" -eq "$REQS" && "$FAIL" -eq 0 && "$SYN" -ge 1 && "$SYN" -lt "$REQS" ]] && KEEP=PASS
echo "KEEP_CONN_REGRESSION=$KEEP OK=$OK FAIL=$FAIL REQS=$REQS SYN=$SYN" \
  | tee "$STAGE_DIR/STAGES/KEEP_CONN.txt"
[[ "$KEEP" == PASS ]] || { echo "FAIL: KEEP_CONN"; exit 2; }

{
  echo "WIP=V044_PHASE6_FASTCGI_MAX_CONNECTIONS_CONFIG_HONESTY_CLOSE"
  echo "ADR=042"
  echo "CASE_HINT=P6MAXHONEST-A"
  echo "REJECT_MAX6_PASS=YES"
  echo "REJECT_MAX0_PASS=YES"
  echo "VALUE_1_POLICY=ACCEPTED"
  echo "FAILED_PUBLISH_NO_EXTERNAL_EFFECT=PASS"
  echo "GEN_RELOAD_MAX1_PASS=YES"
  echo "DYNAMIC_PHP_PASS=YES"
  echo "KEEP_CONN_REGRESSION=$KEEP"
  echo "KEEP_CONN_REQUESTS=$REQS"
  echo "KEEP_CONN_BACKEND_ACCEPTS=$SYN"
  echo "REAL_PHP_FPM=PASS"
  echo "PHP_FPM_MAX_CHILDREN=$FPM_CHILDREN"
} | tee "$STAGE_DIR/SUMMARY.txt"

echo "P6MAXHONEST_DONE KEEP=$KEEP"
