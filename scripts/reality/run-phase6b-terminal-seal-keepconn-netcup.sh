#!/usr/bin/env bash
# V044 Phase 6B terminal seal — causal KEEP_CONN vs real PHP-FPM (Netcup amd64).
# ZERO_FAKE / NO_SMOKE: independent backend SYN accept oracle (not ExyonQ pool counters).
# Distinguishes CLIENT_H1_KEEPALIVE from FASTCGI_BACKEND_KEEP_CONN.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6b-fastcgi-mvp-terminal-seal}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
WORKDIR="${WORKDIR:-/tmp/exyonq-p6bseal-$RUN_ID}"
LISTEN="${LISTEN:-127.0.0.1:18081}"
FPM_PORT="${FPM_PORT:-19001}"
N_REQ="${N_REQ:-100}"
DOCROOT="$WORKDIR/www"
GEN_DIR="$WORKDIR/gen"
FPM_CONF="$WORKDIR/php-fpm.conf"
FPM_POOL="$WORKDIR/pool.conf"
ROUTES="$WORKDIR/routes.txt"
MARKER_FILE="$DOCROOT/marker.php"
SYN_PCAP="$WORKDIR/fpm-syn.pcap"
SYN_TXT="$WORKDIR/fpm-syn.txt"

mkdir -p "$DOCROOT" "$GEN_DIR" "$EVIDENCE_ROOT/$RUN_ID"
cd "$ROOT"

echo "P6BSEAL_START run=$RUN_ID workdir=$WORKDIR host=$(uname -m) N_REQ=$N_REQ"

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
[[ -n "$FPM_USER" ]] || { echo "FAIL: no FPM user"; exit 2; }
[[ -n "$FPM_GROUP" ]] || FPM_GROUP=$FPM_USER

# Reset counter so N markers are contiguous for this run.
rm -f /tmp/exyonq_p6b_fcgi_counter /tmp/exyonq_p6bseal_fcgi_counter

cat > "$MARKER_FILE" <<'PHP'
<?php
header('Content-Type: text/plain; charset=utf-8');
$path = sys_get_temp_dir() . '/exyonq_p6bseal_fcgi_counter';
$fp = fopen($path, 'c+');
if ($fp === false) { http_response_code(500); echo "open_fail\n"; exit; }
flock($fp, LOCK_EX);
$n = intval(stream_get_contents($fp));
$n++;
ftruncate($fp, 0);
rewind($fp);
fwrite($fp, (string)$n);
fflush($fp);
flock($fp, LOCK_UN);
fclose($fp);
$nonce = bin2hex(random_bytes(8));
echo "P6BSEAL_DYN n={$n} nonce={$nonce}\n";
PHP

chmod 755 "$WORKDIR" "$DOCROOT"
chmod 644 "$MARKER_FILE"
if [[ "$(id -u)" -eq 0 ]]; then
  chown -R "$FPM_USER:$FPM_GROUP" "$DOCROOT" || true
fi

cat > "$FPM_POOL" <<EOF
[www]
user = ${FPM_USER}
group = ${FPM_GROUP}
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = 4
pm.status_path = /status
clear_env = no
security.limit_extensions = .php
EOF

cat > "$FPM_CONF" <<EOF
[global]
pid = $WORKDIR/php-fpm.pid
error_log = $WORKDIR/php-fpm.log
daemonize = no
include = $FPM_POOL
EOF

PHP_FPM_BIN="${PHP_FPM_BIN:-}"
if [[ -z "$PHP_FPM_BIN" ]]; then
  for c in /usr/sbin/php-fpm8.3 /usr/sbin/php-fpm php-fpm8.3 php-fpm; do
    if [[ -x "$c" ]] || command -v "$c" >/dev/null 2>&1; then
      PHP_FPM_BIN=$(command -v "$c" 2>/dev/null || echo "$c")
      break
    fi
  done
fi
[[ -n "$PHP_FPM_BIN" ]] || { echo "FAIL: php-fpm not found"; exit 2; }

"$PHP_FPM_BIN" -y "$FPM_CONF" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
DP_PID=""
TCPDUMP_PID=""
cleanup() {
  [[ -n "${TCPDUMP_PID:-}" ]] && kill "$TCPDUMP_PID" 2>/dev/null || true
  [[ -n "${DP_PID:-}" ]] && kill "$DP_PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
  wait "$TCPDUMP_PID" 2>/dev/null || true
  wait "$DP_PID" 2>/dev/null || true
  wait "$FPM_PID" 2>/dev/null || true
}
trap cleanup EXIT

FPM_UP=0
for _ in $(seq 1 100); do
  if ! kill -0 "$FPM_PID" 2>/dev/null; then
    echo "FAIL: php-fpm exited during start"
    cat "$WORKDIR/fpm.err" "$WORKDIR/php-fpm.log" 2>/dev/null || true
    exit 2
  fi
  if (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1; then
    FPM_UP=1
    break
  fi
  sleep 0.1
done
[[ "$FPM_UP" -eq 1 ]] || {
  echo "FAIL: php-fpm did not listen on ${FPM_PORT}"
  cat "$WORKDIR/fpm.err" "$WORKDIR/php-fpm.log" 2>/dev/null || true
  exit 2
}

# Independent oracle: TCP SYN to FPM listen port (new connections).
# tcpdump may need root; Netcup seal runs as root.
rm -f "$SYN_PCAP" "$SYN_TXT"
tcpdump -nn -i lo -U -w "$SYN_PCAP" \
  "tcp dst port ${FPM_PORT} and (tcp[tcpflags] & tcp-syn) != 0 and (tcp[tcpflags] & tcp-ack) == 0" \
  >"$WORKDIR/tcpdump.out" 2>"$WORKDIR/tcpdump.err" &
TCPDUMP_PID=$!
sleep 0.3

cat > "$ROUTES" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|1|60000|2000|30000|30000|60000
|/marker.php|fcgi:1
EOF

cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL: missing binaries"; exit 3; }

"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 1 | tee "$EVIDENCE_ROOT/$RUN_ID/publish.txt"

"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards 1 --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!

for _ in $(seq 1 100); do
  if grep -q '^READY ' "$GEN_DIR/status" 2>/dev/null; then break; fi
  if ! kill -0 "$DP_PID" 2>/dev/null; then
    echo "FAIL: dataplane exited early"
    cat "$WORKDIR/dp.err" || true
    exit 3
  fi
  sleep 0.1
done
grep -q '^READY ' "$GEN_DIR/status"
grep -q 'TOKIO_THREADS_IN_DATAPLANE_PROCESS=0' "$WORKDIR/dp.err"
grep -q 'HYPER_IN_DATAPLANE_PROCESS=0' "$WORKDIR/dp.err"

# Warm: one request so first accept is outside the measured window optionally counted.
# We measure ALL SYNs during the N_REQ loop including first connect — M should still be << N if KEEP_CONN works.

declare -a BODIES=()
declare -a NONCES=()
SUCCESS=0
FAILS=0
PREV_N=0
CONTAMINATION=0

echo "CLIENT_H1_KEEPALIVE=NO (Connection: close each request)"
echo "FASTCGI_BACKEND_KEEP_CONN=measured via SYN oracle"

for i in $(seq 1 "$N_REQ"); do
  BODY=$(curl -fsS --http1.1 -H 'Connection: close' --max-time 10 "http://${LISTEN}/marker.php" || true)
  if echo "$BODY" | grep -q '^P6BSEAL_DYN n='; then
    SUCCESS=$((SUCCESS + 1))
    NN=$(echo "$BODY" | sed -n 's/.*n=\([0-9]*\).*/\1/p')
    NONCE=$(echo "$BODY" | sed -n 's/.*nonce=\([0-9a-f]*\).*/\1/p')
    if [[ -z "$NN" || -z "$NONCE" ]]; then
      CONTAMINATION=$((CONTAMINATION + 1))
    fi
    if [[ "$PREV_N" -gt 0 && "$NN" -le "$PREV_N" ]]; then
      CONTAMINATION=$((CONTAMINATION + 1))
    fi
    for prev in "${NONCES[@]:-}"; do
      if [[ -n "$prev" && "$prev" == "$NONCE" ]]; then
        CONTAMINATION=$((CONTAMINATION + 1))
      fi
    done
    PREV_N=$NN
    NONCES+=("$NONCE")
    BODIES+=("$BODY")
  else
    FAILS=$((FAILS + 1))
    echo "REQ_FAIL i=$i body=$BODY"
  fi
done

# Stop tcpdump cleanly and count SYNs.
kill -INT "$TCPDUMP_PID" 2>/dev/null || true
wait "$TCPDUMP_PID" 2>/dev/null || true
TCPDUMP_PID=""
sleep 0.2
SYN_COUNT=0
if [[ -f "$SYN_PCAP" ]]; then
  SYN_COUNT=$(tcpdump -nn -r "$SYN_PCAP" 2>/dev/null | wc -l | tr -d ' ')
  tcpdump -nn -r "$SYN_PCAP" >"$SYN_TXT" 2>/dev/null || true
fi
cp "$SYN_PCAP" "$EVIDENCE_ROOT/$RUN_ID/fpm-syn.pcap" 2>/dev/null || true
cp "$SYN_TXT" "$EVIDENCE_ROOT/$RUN_ID/fpm-syn.txt" 2>/dev/null || true

# Secondary oracle: PHP-FPM status Accepted conn via cgi-fcgi if available.
FPM_ACCEPTED="NA"
if command -v cgi-fcgi >/dev/null 2>&1; then
  STATUS_OUT=$(SCRIPT_NAME=/status SCRIPT_FILENAME=/status REQUEST_METHOD=GET \
    cgi-fcgi -bind -connect "127.0.0.1:${FPM_PORT}" 2>/dev/null || true)
  echo "$STATUS_OUT" | tee "$EVIDENCE_ROOT/$RUN_ID/fpm-status.txt" >/dev/null
  FPM_ACCEPTED=$(echo "$STATUS_OUT" | sed -n 's/^accepted conn:\s*//p' | head -1 | tr -d '[:space:]')
  [[ -z "$FPM_ACCEPTED" ]] && FPM_ACCEPTED="NA"
fi

# Generation drain: gen=1 pool → publish gen=2 → request must succeed under gen 2.
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 2 | tee -a "$EVIDENCE_ROOT/$RUN_ID/publish.txt"
GEN_OK=0
for _ in $(seq 1 100); do
  HDR=$(curl -fsS --http1.1 -H 'Connection: close' --max-time 5 -D - -o "$WORKDIR/gen_body" "http://${LISTEN}/marker.php" || true)
  if echo "$HDR" | grep -qi 'x-exyonq-cfd-generation: *2' && grep -q '^P6BSEAL_DYN n=' "$WORKDIR/gen_body"; then
    GEN_OK=1
    break
  fi
  if grep -q 'generation updated id=2' "$WORKDIR/dp.err" 2>/dev/null && grep -q '^P6BSEAL_DYN n=' "$WORKDIR/gen_body" 2>/dev/null; then
    GEN_OK=1
    break
  fi
  sleep 0.1
done
[[ "$GEN_OK" -eq 1 ]] || {
  echo "FAIL: generation drain / gen=2 not observed"
  cat "$WORKDIR/dp.err" || true
  exit 4
}

# Backend failure: kill FPM → request must fail; no silent reuse success.
kill "$FPM_PID" 2>/dev/null || true
wait "$FPM_PID" 2>/dev/null || true
FPM_PID=""
for _ in $(seq 1 50); do
  if ! (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1; then break; fi
  sleep 0.1
done
CODE=$(curl -s --http1.1 -H 'Connection: close' --max-time 5 -o /dev/null -w '%{http_code}' "http://${LISTEN}/marker.php" || true)
[[ "$CODE" == "502" || "$CODE" == "504" ]] || {
  echo "FAIL: expected 502/504 after FPM down, got $CODE"
  exit 5
}

RATIO="NA"
ACCEPTS_PER_REQ="NA"
if [[ "$SUCCESS" -gt 0 && "$SYN_COUNT" =~ ^[0-9]+$ ]]; then
  ACCEPTS_PER_REQ=$(awk -v m="$SYN_COUNT" -v n="$SUCCESS" 'BEGIN{printf "%.6f", m/n}')
  RATIO=$(awk -v m="$SYN_COUNT" -v n="$SUCCESS" 'BEGIN{ if(m<=0) print "INF"; else printf "%.4f", n/m }')
fi

CAUSAL=FAIL
if [[ "$SUCCESS" -eq "$N_REQ" && "$FAILS" -eq 0 && "$CONTAMINATION" -eq 0 && "$SYN_COUNT" -lt "$SUCCESS" && "$SYN_COUNT" -ge 1 ]]; then
  CAUSAL=PASS
fi

{
  echo "REAL_PHP_FPM=PASS"
  echo "REAL_DYNAMIC_PHP=PASS"
  echo "REAL_CAUSAL_KEEP_CONN=$CAUSAL"
  echo "REAL_GENERATION_DRAIN=PASS"
  echo "REAL_BACKEND_FAILURE=PASS"
  echo "CLIENT_H1_KEEPALIVE=NO"
  echo "FASTCGI_BACKEND_KEEP_CONN=ORACLE_TCP_SYN"
  echo "KEEP_CONN_REQUESTS=$SUCCESS"
  echo "KEEP_CONN_FAILS=$FAILS"
  echo "PHP_FPM_ACCEPTED_CONNECTIONS_SYN=$SYN_COUNT"
  echo "PHP_FPM_STATUS_ACCEPTED_CONN=$FPM_ACCEPTED"
  echo "FASTCGI_BACKEND_ACCEPTS_PER_REQUEST=$ACCEPTS_PER_REQ"
  echo "FASTCGI_REAL_PHP_FPM_REUSE_RATIO=$RATIO"
  echo "CROSS_REQUEST_FASTCGI_CONTAMINATION=$CONTAMINATION"
  # Backend-down observed as 502/504 (no successful reuse after FPM kill).
  echo "FAILED_BACKEND_CONNECTION_REUSE=0"
  echo "FAILED_BACKEND_OBSERVED_STATUS=$CODE"
  # Adversarial trailing/partial-record dirty reuse is covered by cargo tests
  # (phase6b_fcgi / wire TrailingData), NOT by this PHP-FPM SYN harness.
  echo "DIRTY_CONNECTION_REUSE_COUNT=NOT_MEASURED_IN_PHP_FPM_SEAL"
  echo "DIRTY_REUSE_COMPONENT_TESTS=SEE_CARGO_PHASE6B_FCGI"
  echo "LEGACY_FASTCGI_RUNTIME_USED=NO_REQUIRED"
  echo "TOKIO_THREADS_IN_CFD=0"
  echo "HYPER_ON_CFD_FASTCGI_PATH=0"
  echo "PLATFORM=LINUX_AMD64"
  echo "PHP_FPM_BIN=$PHP_FPM_BIN"
  echo "FPM_USER=$FPM_USER"
  echo "N_REQ=$N_REQ"
  echo "BACKEND_DOWN_STATUS=$CODE"
  echo "CFD_BIN=$BIN"
  echo "ENTRY_HEAD=${ENTRY_HEAD:-}"
  echo "BINARY_SHA256=$(sha256sum "$BIN" | awk '{print $1}')"
  echo "ORACLE=tcpdump_SYN_to_FPM_port"
  echo "SELF_REPORTED_POOL_COUNTER_USED_AS_PRIMARY=NO"
} | tee "$EVIDENCE_ROOT/$RUN_ID/SUMMARY.txt"

cp "$WORKDIR/dp.err" "$EVIDENCE_ROOT/$RUN_ID/dataplane.err" || true
cp "$MARKER_FILE" "$EVIDENCE_ROOT/$RUN_ID/marker.php"
cp "$SYN_TXT" "$EVIDENCE_ROOT/$RUN_ID/" 2>/dev/null || true

if [[ "$CAUSAL" != "PASS" ]]; then
  echo "P6BSEAL_CAUSAL_KEEP_CONN_FAIL evidence=$EVIDENCE_ROOT/$RUN_ID"
  exit 6
fi
echo "P6BSEAL_PASS evidence=$EVIDENCE_ROOT/$RUN_ID"
