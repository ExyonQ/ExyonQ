#!/usr/bin/env bash
# V044 — FastCGI max_connections geometry matrix (Netcup amd64).
# HISTORICAL (P6MAX-C): after ADR-042, CFD max_conn!=1 is rejected at publish; do not re-run for current product honesty.
# Superseding harness: scripts/reality/run-fcgi-max-connections-honesty-netcup.sh
# WIP: V044_PHASE6_FASTCGI_MAX_CONNECTIONS_STRONG_ORACLE_CLOSE
# PRODUCT_MUTATION=NO. Measures ESTAB + PHP overlap; does NOT invent concurrency.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6-fastcgi-max-connections-strong-oracle-close}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
WORKDIR="${WORKDIR:-/tmp/exyonq-p6max-$RUN_ID}"
LISTEN="${LISTEN:-127.0.0.1:18107}"
FPM_PORT="${FPM_PORT:-19107}"
FPM_CHILDREN="${FPM_CHILDREN:-16}"
IDLE_MS="${IDLE_MS:-60000}"
SLOW_MS="${SLOW_MS:-800}"
CLIENT_CONC="${CLIENT_CONC:-6}"
SHARDS_GEO1="${SHARDS_GEO1:-1}"
SHARDS_GEO2="${SHARDS_GEO2:-4}"
KEEP_REQS="${KEEP_REQS:-200}"
ENTRY_HEAD="${ENTRY_HEAD:-}"
ENTRY_TREE="${ENTRY_TREE:-}"
DOCROOT="$WORKDIR/www"
GEN_DIR="$WORKDIR/gen"
LEDGER="$WORKDIR/php-ledger.ndjson"
ROUTES="$WORKDIR/routes.txt"
STAGE_DIR="$EVIDENCE_ROOT/$RUN_ID"

mkdir -p "$DOCROOT" "$GEN_DIR" "$STAGE_DIR/GEO" "$STAGE_DIR/FPM" "$STAGE_DIR/KEEP_CONN" \
  "$STAGE_DIR/SOURCE_MAP" "$STAGE_DIR/ORACLE_SYN"
cd "$ROOT"

FPM_USER="${FPM_USER:-}"
FPM_GROUP="${FPM_GROUP:-}"
if [[ -z "$FPM_USER" ]]; then
  if [[ "$(id -u)" -eq 0 ]]; then
    for cand in www-data nginx nobody; do
      if id -u "$cand" >/dev/null 2>&1; then
        FPM_USER=$cand; FPM_GROUP=$(id -gn "$cand"); break
      fi
    done
  else
    FPM_USER=$(id -un); FPM_GROUP=$(id -gn)
  fi
fi

: >"$LEDGER"
touch "$LEDGER"; chmod 666 "$LEDGER" || true
# Embed absolute ledger path (FPM does not inherit harness env).
cat >"$DOCROOT/ledger.php" <<PHP
<?php
header('Content-Type: text/plain; charset=utf-8');
\$ledger = '${LEDGER}';
\$rid = isset(\$_GET['rid']) ? preg_replace('/[^a-zA-Z0-9_-]/', '', \$_GET['rid']) : 'na';
\$ms = isset(\$_GET['ms']) ? max(0, min(30000, intval(\$_GET['ms']))) : 2000;
\$t0 = microtime(true);
\$pid = getmypid();
file_put_contents(\$ledger, json_encode(['ev'=>'start','rid'=>\$rid,'pid'=>\$pid,'t'=>\$t0])."\\n", FILE_APPEND|LOCK_EX);
usleep(\$ms * 1000);
\$t1 = microtime(true);
file_put_contents(\$ledger, json_encode(['ev'=>'end','rid'=>\$rid,'pid'=>\$pid,'t'=>\$t1,'dur_ms'=>(\$t1-\$t0)*1000])."\\n", FILE_APPEND|LOCK_EX);
echo "LEDGER rid={\$rid} pid={\$pid} nonce=" . bin2hex(random_bytes(8)) . "\\n";
PHP
cat >"$DOCROOT/marker.php" <<'PHP'
<?php
header('Content-Type: text/plain; charset=utf-8');
echo "P6MAX_DYN nonce=" . bin2hex(random_bytes(8)) . "\n";
PHP
chmod 644 "$DOCROOT"/*.php
[[ "$(id -u)" -eq 0 ]] && chown -R "$FPM_USER:$FPM_GROUP" "$DOCROOT" || true

write_fpm() {
  cat >"$WORKDIR/pool.conf" <<EOF
[www]
user = ${FPM_USER}
group = ${FPM_GROUP}
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = ${FPM_CHILDREN}
security.limit_extensions = .php
EOF
  cat >"$WORKDIR/php-fpm.conf" <<EOF
[global]
pid = $WORKDIR/php-fpm.pid
error_log = $WORKDIR/php-fpm.log
daemonize = no
include = $WORKDIR/pool.conf
EOF
}

write_routes() {
  local maxc="$1"
  cat >"$ROUTES" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|${maxc}|${IDLE_MS}|2000|30000|30000|60000
|/ledger.php|fcgi:1
|/marker.php|fcgi:1
EOF
}

ss_estab() {
  ss -Htan state established "dport = :${FPM_PORT}" 2>/dev/null | wc -l | tr -d ' '
}

php_overlap_peak() {
  # Parse ledger starts/ends → peak concurrent PHP executions
  python3 - <<'PY' "$LEDGER"
import json,sys
path=sys.argv[1]
ev=[]
with open(path) as f:
  for line in f:
    line=line.strip()
    if not line: continue
    o=json.loads(line)
    if o['ev']=='start': ev.append((o['t'],+1))
    elif o['ev']=='end': ev.append((o['t'],-1))
ev.sort()
cur=peak=0
for _,d in ev:
  cur+=d
  peak=max(peak,cur)
print(peak)
PY
}

run_hold() {
  local label="$1" maxc="$2" shards="$3" conc="$4"
  local out="$STAGE_DIR/GEO/${label}.txt"
  : >"$LEDGER"
  write_routes "$maxc"
  "$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id "$GEN_ID" >/dev/null
  GEN_ID=$((GEN_ID + 1))
  sleep 0.4
  local pids=()
  local i
  for i in $(seq 1 "$conc"); do
    (
      curl -fsS --http1.1 -H 'Connection: close' --max-time 25 \
        "http://${LISTEN}/ledger.php?ms=${SLOW_MS}&rid=${label}-$i" \
        -o "$WORKDIR/${label}-$i.out" 2>/dev/null || true
    ) &
    pids+=($!)
  done
  local peak=0 c
  for _ in $(seq 1 12); do
    c=$(ss_estab)
    [[ "$c" -gt "$peak" ]] && peak=$c
    sleep 0.2
  done
  wait "${pids[@]}" 2>/dev/null || true
  local php_peak
  php_peak=$(php_overlap_peak)
  {
    echo "LABEL=$label"
    echo "MAX_CONNECTIONS=$maxc"
    echo "SHARDS=$shards"
    echo "CLIENT_CONC=$conc"
    echo "MAX_BACKEND_ESTAB=$peak"
    echo "MAX_PHP_EXECUTIONS_OVERLAP=$php_peak"
    echo "SLOW_MS=$SLOW_MS"
  } >"$out"
  # stdout: ONLY two integers for caller capture (no tee pollution)
  echo "$peak $php_peak"
}

PHP_FPM_BIN=""
for c in /usr/sbin/php-fpm8.3 /usr/sbin/php-fpm php-fpm; do
  if [[ -x "$c" ]] || command -v "$c" >/dev/null 2>&1; then
    PHP_FPM_BIN=$(command -v "$c" 2>/dev/null || echo "$c"); break
  fi
done
[[ -n "$PHP_FPM_BIN" ]] || { echo "FAIL: php-fpm"; exit 2; }

FPM_PID="" DP_PID=""
cleanup() {
  [[ -n "${DP_PID:-}" ]] && kill "$DP_PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
  [[ -f "$WORKDIR/php-fpm.pid" ]] && kill "$(cat "$WORKDIR/php-fpm.pid")" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
BINARY_SHA256=$(sha256sum "$BIN" | awk '{print $1}')
echo "$BINARY_SHA256" | tee "$STAGE_DIR/BINARY_SHA256.txt"

write_fpm
cp "$WORKDIR/pool.conf" "$STAGE_DIR/FPM/"
"$PHP_FPM_BIN" -y "$WORKDIR/php-fpm.conf" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
for _ in $(seq 1 100); do
  (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1 && break
  sleep 0.1
done

GEN_ID=1
write_routes 1
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id "$GEN_ID" | tee "$STAGE_DIR/FPM/publish.txt"
GEN_ID=$((GEN_ID + 1))

# ---- GEO-1: shards=1, max=1 then 2 then 6 ----
start_dp() {
  local shards="$1"
  [[ -n "${DP_PID:-}" ]] && kill "$DP_PID" 2>/dev/null || true
  wait "$DP_PID" 2>/dev/null || true
  "$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$shards" --schema-version 2 \
    >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
  DP_PID=$!
  for _ in $(seq 1 150); do
    grep -q READY "$WORKDIR/dp.err" 2>/dev/null && break
    kill -0 "$DP_PID" 2>/dev/null || { cat "$WORKDIR/dp.err"; exit 2; }
    sleep 0.1
  done
}

start_dp "$SHARDS_GEO1"
{
  echo "WIP=V044_PHASE6_FASTCGI_MAX_CONNECTIONS_STRONG_ORACLE_CLOSE"
  echo "RUN_ID=$RUN_ID"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "BINARY_SHA256=$BINARY_SHA256"
  echo "PHP_FPM_MAX_CHILDREN=$FPM_CHILDREN"
  echo "PRODUCT_MUTATION=NO"
} | tee "$STAGE_DIR/MANIFEST.txt"

echo "=== GEO-1 shards=$SHARDS_GEO1 ==="
GEO1_MAX1=$(run_hold GEO1_max1 1 "$SHARDS_GEO1" "$CLIENT_CONC")
GEO1_MAX2=$(run_hold GEO1_max2 2 "$SHARDS_GEO1" "$CLIENT_CONC")
GEO1_MAX6=$(run_hold GEO1_max6 6 "$SHARDS_GEO1" "$CLIENT_CONC")
GEO1_MAX1_ESTAB=${GEO1_MAX1%% *}; GEO1_MAX1_PHP=${GEO1_MAX1##* }
GEO1_MAX2_ESTAB=${GEO1_MAX2%% *}; GEO1_MAX2_PHP=${GEO1_MAX2##* }
GEO1_MAX6_ESTAB=${GEO1_MAX6%% *}; GEO1_MAX6_PHP=${GEO1_MAX6##* }
echo "GEO1_max1 ESTAB=$GEO1_MAX1_ESTAB PHP=$GEO1_MAX1_PHP" | tee -a "$STAGE_DIR/GEO/console.txt"
echo "GEO1_max2 ESTAB=$GEO1_MAX2_ESTAB PHP=$GEO1_MAX2_PHP" | tee -a "$STAGE_DIR/GEO/console.txt"
echo "GEO1_max6 ESTAB=$GEO1_MAX6_ESTAB PHP=$GEO1_MAX6_PHP" | tee -a "$STAGE_DIR/GEO/console.txt"

echo "=== GEO-2 shards=$SHARDS_GEO2 max=1 (SHARD_SPILL_NOT_MAX) ==="
# republish gen for new dp
kill "$DP_PID" 2>/dev/null || true; wait "$DP_PID" 2>/dev/null || true; DP_PID=""
write_routes 1
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id "$GEN_ID" >/dev/null
GEN_ID=$((GEN_ID + 1))
start_dp "$SHARDS_GEO2"
GEO2=$(run_hold GEO2_shards_max1 1 "$SHARDS_GEO2" "$((SHARDS_GEO2 * 2))")
GEO2_ESTAB=${GEO2%% *}; GEO2_PHP=${GEO2##* }
echo "GEO2 ESTAB=$GEO2_ESTAB PHP=$GEO2_PHP" | tee -a "$STAGE_DIR/GEO/console.txt"

echo "=== KEEP_CONN regression shards=1 ==="
kill "$DP_PID" 2>/dev/null || true; wait "$DP_PID" 2>/dev/null || true; DP_PID=""
write_routes 1
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id "$GEN_ID" >/dev/null
GEN_ID=$((GEN_ID + 1))
start_dp 1
PCAP="$STAGE_DIR/ORACLE_SYN/keep.pcap"
tcpdump -i lo -nn -w "$PCAP" "tcp and dst port $FPM_PORT and tcp[tcpflags] & tcp-syn != 0" >/dev/null 2>&1 &
TCPDUMP_PID=$!
sleep 0.4
OK=0
for i in $(seq 1 "$KEEP_REQS"); do
  curl -fsS -o /dev/null --http1.1 -H 'Connection: close' "http://${LISTEN}/marker.php?rid=k$i" && OK=$((OK+1)) || true
done
sleep 0.3
kill "$TCPDUMP_PID" 2>/dev/null || true; wait "$TCPDUMP_PID" 2>/dev/null || true
SYN=$(tcpdump -nn -r "$PCAP" 2>/dev/null | wc -l | tr -d ' ')
KEEP=FAIL
[[ "$OK" -eq "$KEEP_REQS" && "$SYN" -ge 1 && "$SYN" -lt "$KEEP_REQS" ]] && KEEP=PASS
echo "KEEP_CONN_REGRESSION=$KEEP OK=$OK SYN=$SYN" | tee "$STAGE_DIR/KEEP_CONN/SUMMARY.txt"

# Classification helpers
SERIAL_CONFIRMED=NO
[[ "$GEO1_MAX1_ESTAB" -eq 1 && "$GEO1_MAX2_ESTAB" -eq 1 && "$GEO1_MAX6_ESTAB" -eq 1 ]] && SERIAL_CONFIRMED=YES
MAX_DISTINGUISHABLE=NO
[[ "$GEO1_MAX1_ESTAB" != "$GEO1_MAX6_ESTAB" || "$GEO1_MAX2_ESTAB" != "$GEO1_MAX6_ESTAB" ]] && MAX_DISTINGUISHABLE=YES

{
  echo "SERIAL_CONFIRMED=$SERIAL_CONFIRMED"
  echo "MAX_DISTINGUISHABLE_BY_ESTAB=$MAX_DISTINGUISHABLE"
  echo "MAX_CONTROL_1_BACKEND_ESTAB=$GEO1_MAX1_ESTAB"
  echo "MAX_CONTROL_1_PHP_OVERLAP=$GEO1_MAX1_PHP"
  echo "MAX_2_BACKEND_ESTAB=$GEO1_MAX2_ESTAB"
  echo "MAX_2_PHP_OVERLAP=$GEO1_MAX2_PHP"
  echo "MAX_6_BACKEND_ESTAB=$GEO1_MAX6_ESTAB"
  echo "MAX_6_PHP_OVERLAP=$GEO1_MAX6_PHP"
  echo "GEO2_SHARDS=$SHARDS_GEO2"
  echo "GEO2_ESTAB=$GEO2_ESTAB"
  echo "GEO2_PHP_OVERLAP=$GEO2_PHP"
  echo "GEO2_LABEL=SHARD_SPILL_NOT_MAX_CONNECTIONS"
  echo "KEEP_CONN_REGRESSION=$KEEP"
  echo "KEEP_CONN_REQUESTS=$OK"
  echo "KEEP_CONN_BACKEND_ACCEPTS=$SYN"
  echo "BINARY_SHA256=$BINARY_SHA256"
  if [[ "$SERIAL_CONFIRMED" == YES && "$MAX_DISTINGUISHABLE" == NO ]]; then
    echo "CASE_HINT=P6MAX-C"
    echo "MAX_GT_1_RUNTIME_EFFECT_POSSIBLE=NO"
    echo "MAX_CONNECTIONS_GT1_SEMANTICS=INOPERATIVE_CURRENTLY"
  else
    echo "CASE_HINT=REVIEW"
  fi
} | tee "$STAGE_DIR/SUMMARY.txt"

cp "$LEDGER" "$STAGE_DIR/GEO/php-ledger.ndjson" || true
echo "P6MAX_GEO_DONE"
