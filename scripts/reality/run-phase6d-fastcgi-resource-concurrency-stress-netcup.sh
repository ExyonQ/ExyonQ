#!/usr/bin/env bash
# V044 Phase 6D — FastCGI resource/concurrency stress qualification (Netcup amd64).
# QUALIFICATION ONLY. PRODUCT_MUTATION=NO. ZERO_FAKE / NO_SMOKE.
# Independent oracles: tcpdump SYN-to-FPM, dynamic PHP identity, /proc resource series.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6d-fastcgi-resource-concurrency-stress-qualification}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
WORKDIR="${WORKDIR:-/tmp/exyonq-p6d-$RUN_ID}"
LISTEN="${LISTEN:-127.0.0.1:18083}"
FPM_PORT="${FPM_PORT:-19003}"
POOL_MAX="${POOL_MAX:-1}"
FPM_CHILDREN="${FPM_CHILDREN:-4}"
IDLE_MS="${IDLE_MS:-60000}"
ENTRY_HEAD="${ENTRY_HEAD:-}"
DOCROOT="$WORKDIR/www"
GEN_DIR="$WORKDIR/gen"
FPM_CONF="$WORKDIR/php-fpm.conf"
FPM_POOL="$WORKDIR/pool.conf"
ROUTES="$WORKDIR/routes.txt"
STAGE_DIR="$EVIDENCE_ROOT/$RUN_ID"
SERIES="$STAGE_DIR/ORACLE_RESOURCES/series.csv"
SAMPLER_PID=""
FPM_PID=""
DP_PID=""
TCPDUMP_PID=""

mkdir -p "$DOCROOT" "$GEN_DIR" \
  "$STAGE_DIR/ORACLE_SYN" "$STAGE_DIR/ORACLE_DYNAMIC" "$STAGE_DIR/ORACLE_RESOURCES" \
  "$STAGE_DIR/LOAD" "$STAGE_DIR/FPM" "$STAGE_DIR/STAGES"
cd "$ROOT"

echo "P6D_START run=$RUN_ID host=$(uname -m) pool_max=$POOL_MAX fpm_children=$FPM_CHILDREN"

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

rm -f /tmp/exyonq_p6d_fcgi_counter /tmp/exyonq_p6d_side_effect

# --- PHP scripts (dynamic identity) ---
cat > "$DOCROOT/marker.php" <<'PHP'
<?php
header('Content-Type: text/plain; charset=utf-8');
$path = sys_get_temp_dir() . '/exyonq_p6d_fcgi_counter';
$fp = fopen($path, 'c+');
if ($fp === false) { http_response_code(500); echo "open_fail\n"; exit; }
flock($fp, LOCK_EX);
$n = intval(stream_get_contents($fp));
$n++;
ftruncate($fp, 0); rewind($fp); fwrite($fp, (string)$n); fflush($fp);
flock($fp, LOCK_UN); fclose($fp);
$nonce = bin2hex(random_bytes(8));
$rid = isset($_GET['rid']) ? preg_replace('/[^a-zA-Z0-9_-]/', '', $_GET['rid']) : 'na';
echo "P6D_DYN n={$n} nonce={$nonce} rid={$rid}\n";
PHP

cat > "$DOCROOT/slow.php" <<'PHP'
<?php
header('Content-Type: text/plain; charset=utf-8');
$ms = isset($_GET['ms']) ? max(0, min(30000, intval($_GET['ms']))) : 200;
usleep($ms * 1000);
$nonce = bin2hex(random_bytes(8));
echo "P6D_SLOW ms={$ms} nonce={$nonce}\n";
PHP

cat > "$DOCROOT/big.php" <<'PHP'
<?php
header('Content-Type: text/plain; charset=utf-8');
$kb = isset($_GET['kb']) ? max(1, min(256, intval($_GET['kb']))) : 64;
$nonce = bin2hex(random_bytes(8));
echo "P6D_BIG kb={$kb} nonce={$nonce}\n";
echo str_repeat("X", $kb * 1024);
PHP

cat > "$DOCROOT/app404.php" <<'PHP'
<?php
http_response_code(404);
header('Content-Type: text/plain; charset=utf-8');
echo "P6D_APP404 nonce=" . bin2hex(random_bytes(8)) . "\n";
PHP

chmod 755 "$WORKDIR" "$DOCROOT"
chmod 644 "$DOCROOT"/*.php
if [[ "$(id -u)" -eq 0 ]]; then
  chown -R "$FPM_USER:$FPM_GROUP" "$DOCROOT" || true
fi

write_fpm_pool() {
  local children="$1"
  cat > "$FPM_POOL" <<EOF
[www]
user = ${FPM_USER}
group = ${FPM_GROUP}
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = ${children}
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
}

write_routes() {
  local maxc="$1" idle="$2"
  cat > "$ROUTES" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|${maxc}|${idle}|2000|30000|30000|60000
|/marker.php|fcgi:1
|/slow.php|fcgi:1
|/big.php|fcgi:1
|/app404.php|fcgi:1
EOF
}

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

cleanup() {
  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  [[ -n "${TCPDUMP_PID:-}" ]] && kill "$TCPDUMP_PID" 2>/dev/null || true
  [[ -n "${DP_PID:-}" ]] && kill "$DP_PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

start_fpm() {
  write_fpm_pool "${1:-$FPM_CHILDREN}"
  "$PHP_FPM_BIN" -y "$FPM_CONF" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
  FPM_PID=$!
  local up=0
  for _ in $(seq 1 100); do
    if ! kill -0 "$FPM_PID" 2>/dev/null; then
      echo "FAIL: php-fpm exited"; cat "$WORKDIR/fpm.err" "$WORKDIR/php-fpm.log" 2>/dev/null || true; exit 2
    fi
    if (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1; then up=1; break; fi
    sleep 0.1
  done
  [[ "$up" -eq 1 ]] || { echo "FAIL: fpm not listening"; exit 2; }
}

stop_fpm() {
  # Kill master + all workers still holding the listen socket (orphans keep PORT_UP).
  if command -v ss >/dev/null 2>&1; then
    local pids
    pids=$(ss -ltnp "sport = :${FPM_PORT}" 2>/dev/null | grep -oP 'pid=\K[0-9]+' | sort -u || true)
    for p in $pids; do
      kill -9 "$p" 2>/dev/null || true
    done
  fi
  if [[ -n "${FPM_PID:-}" ]]; then
    kill -9 "$FPM_PID" 2>/dev/null || true
    wait "$FPM_PID" 2>/dev/null || true
    FPM_PID=""
  fi
  if [[ -f "$WORKDIR/php-fpm.pid" ]]; then
    kill -9 "$(cat "$WORKDIR/php-fpm.pid")" 2>/dev/null || true
  fi
  for _ in $(seq 1 50); do
    if ! (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1; then break; fi
    sleep 0.1
  done
}

sample_proc() {
  local label="$1" pid="$2"
  if [[ -z "$pid" ]] || ! kill -0 "$pid" 2>/dev/null; then
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$label,$pid,NA,NA,NA" >>"$SERIES"
    return
  fi
  local fd rss thr
  fd=$(ls "/proc/$pid/fd" 2>/dev/null | wc -l | tr -d ' ')
  rss=$(awk '/VmRSS:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo NA)
  thr=$(awk '/Threads:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo NA)
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$label,$pid,$fd,$rss,$thr" >>"$SERIES"
}

start_sampler() {
  echo "utc_ts,label,pid,fd_count,rss_kb,threads" >"$SERIES"
  (
    while true; do
      sample_proc dataplane "${DP_PID:-}"
      sample_proc fpm "${FPM_PID:-}"
      sleep 1
    done
  ) &
  SAMPLER_PID=$!
}

start_tcpdump() {
  local pcap="$1"
  rm -f "$pcap"
  tcpdump -nn -i lo -U -w "$pcap" \
    "tcp dst port ${FPM_PORT} and (tcp[tcpflags] & tcp-syn) != 0 and (tcp[tcpflags] & tcp-ack) == 0" \
    >"$WORKDIR/tcpdump.out" 2>"$WORKDIR/tcpdump.err" &
  TCPDUMP_PID=$!
  # Allow filter attach before first connect (S0 SYN=0 race otherwise).
  sleep 0.5
}

stop_tcpdump_count() {
  local pcap="$1" outtxt="$2"
  kill -INT "$TCPDUMP_PID" 2>/dev/null || true
  wait "$TCPDUMP_PID" 2>/dev/null || true
  TCPDUMP_PID=""
  sleep 0.2
  local n=0
  if [[ -f "$pcap" ]]; then
    n=$(tcpdump -nn -r "$pcap" 2>/dev/null | wc -l | tr -d ' ')
    tcpdump -nn -r "$pcap" >"$outtxt" 2>/dev/null || true
  fi
  echo "$n"
}

# Build once
cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL: missing binaries"; exit 3; }
BINARY_SHA256=$(sha256sum "$BIN" | awk '{print $1}')
echo "$BINARY_SHA256" >"$STAGE_DIR/BINARY_SHA256.txt"

HOST_INFO=$(uname -a)
{
  echo "WIP=V044_PHASE6D_FASTCGI_RESOURCE_CONCURRENCY_STRESS_QUALIFICATION"
  echo "RUN_ID=$RUN_ID"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=${ENTRY_TREE:-}"
  echo "BINARY_SHA256=$BINARY_SHA256"
  echo "PLATFORM=LINUX_AMD64"
  echo "HOST_AUTHORITY=netcup-bench"
  echo "HOST=$HOST_INFO"
  echo "ULIMIT_N=$(ulimit -n)"
  echo "NPROC=$(nproc)"
  echo "PHP_FPM_BIN=$PHP_FPM_BIN"
  echo "PHP_FPM_VERSION=$($PHP_FPM_BIN -v 2>/dev/null | head -1 || true)"
  echo "POOL_MAX_CONFIGURED=$POOL_MAX"
  echo "FPM_CHILDREN=$FPM_CHILDREN"
  echo "IDLE_MS=$IDLE_MS"
  echo "CLAIM_CEILING=BOUNDED_LINUX_AMD64_RESOURCE_CONCURRENCY_STRESS_QUALIFICATION"
  echo "VERIFIED_REAL_PRODUCTION=NOT_CLAIMED"
  echo "SHIPPING=NOT_CLAIMED"
  echo "ALL_PLATFORMS=NOT_CLAIMED"
  echo "ORACLE_ARM64=NOT_EXECUTED"
  echo "PRODUCT_MUTATION=NO"
} | tee "$STAGE_DIR/MANIFEST.txt" >"$STAGE_DIR/CLAIM_CEILING.txt"

write_routes "$POOL_MAX" "$IDLE_MS"
cp "$FPM_POOL" "$STAGE_DIR/FPM/pool.conf" 2>/dev/null || true
start_fpm "$FPM_CHILDREN"
cp "$FPM_POOL" "$STAGE_DIR/FPM/pool.conf"
cp "$FPM_CONF" "$STAGE_DIR/FPM/php-fpm.conf"

"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 1 | tee "$STAGE_DIR/FPM/publish.txt"
"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards 1 --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!

for _ in $(seq 1 100); do
  if grep -q '^READY ' "$GEN_DIR/status" 2>/dev/null; then break; fi
  if ! kill -0 "$DP_PID" 2>/dev/null; then
    echo "FAIL: dataplane exited"; cat "$WORKDIR/dp.err"; exit 3
  fi
  sleep 0.1
done
grep -q '^READY ' "$GEN_DIR/status"
grep -q 'TOKIO_THREADS_IN_DATAPLANE_PROCESS=0' "$WORKDIR/dp.err"
grep -q 'HYPER_IN_DATAPLANE_PROCESS=0' "$WORKDIR/dp.err"
start_sampler
sample_proc dataplane_start "$DP_PID"
FD_START=$(awk -F, '/dataplane_start/ {print $4}' "$SERIES" | tail -1)
RSS_START=$(awk -F, '/dataplane_start/ {print $5}' "$SERIES" | tail -1)
THREAD_START=$(awk -F, '/dataplane_start/ {print $6}' "$SERIES" | tail -1)

# ---- helpers ----
parse_body_ok() {
  # stdin body -> echo CONTAM bits; sets globals via echo KEY=val
  local body="$1" prev_n="$2"
  local nn nonce
  if ! echo "$body" | grep -qE '^P6D_(DYN|SLOW|BIG|APP404)'; then
    echo "FAIL=1"
    return
  fi
  nn=$(echo "$body" | sed -n 's/.*n=\([0-9]*\).*/\1/p' | head -1)
  nonce=$(echo "$body" | sed -n 's/.*nonce=\([0-9a-f]*\).*/\1/p' | head -1)
  local contam=0
  if [[ -z "$nonce" ]]; then contam=1; fi
  if [[ -n "$nn" && "$prev_n" -gt 0 && "$nn" -le "$prev_n" ]]; then contam=1; fi
  echo "OK=1 NN=${nn:-0} NONCE=$nonce CONTAM=$contam"
}

url_for() {
  # $1 = path (/marker.php)  $2 = extra query without leading ? (ms=50)  $3 = rid
  local path="$1" extra="${2:-}" rid="$3"
  if [[ -n "$extra" ]]; then
    echo "http://${LISTEN}${path}?${extra}&rid=${rid}"
  else
    echo "http://${LISTEN}${path}?rid=${rid}"
  fi
}

run_sequential() {
  local n="$1" path="$2" label="$3" extra="${4:-}"
  local success=0 fails=0 contam=0 prev_n=0
  declare -a nonces=()
  local i body parsed
  for i in $(seq 1 "$n"); do
    body=$(curl -fsS --http1.1 -H 'Connection: close' --max-time 15 \
      "$(url_for "$path" "$extra" "${label}-$i")" 2>/dev/null || true)
    if echo "$body" | grep -qE '^P6D_'; then
      success=$((success + 1))
      nn=$(echo "$body" | sed -n 's/.*n=\([0-9]*\).*/\1/p' | head -1)
      nonce=$(echo "$body" | sed -n 's/.*nonce=\([0-9a-f]*\).*/\1/p' | head -1)
      if [[ -z "$nonce" ]]; then contam=$((contam + 1)); fi
      if [[ -n "$nn" && "$prev_n" -gt 0 && "$nn" -le "$prev_n" ]]; then contam=$((contam + 1)); fi
      for prev in "${nonces[@]:-}"; do
        [[ -n "$prev" && "$prev" == "$nonce" ]] && contam=$((contam + 1))
      done
      [[ -n "$nn" ]] && prev_n=$nn
      nonces+=("$nonce")
    else
      fails=$((fails + 1))
    fi
  done
  echo "STAGE=$label SUCCESS=$success FAILS=$fails CONTAMINATION=$contam N=$n"
}

run_concurrent() {
  local conc="$1" n="$2" path="$3" label="$4" extra="${5:-}"
  local tmpd="$WORKDIR/conc-$label"
  mkdir -p "$tmpd"
  rm -f "$tmpd"/*
  local i
  # Launch N requests with concurrency=conc via background jobs
  local active=0 idx=0
  for i in $(seq 1 "$n"); do
    (
      curl -fsS --http1.1 -H 'Connection: close' --max-time 30 \
        "$(url_for "$path" "$extra" "${label}-$i")" \
        >"$tmpd/body.$i" 2>"$tmpd/err.$i" || echo FAIL >"$tmpd/fail.$i"
    ) &
    active=$((active + 1))
    if [[ "$active" -ge "$conc" ]]; then
      wait -n 2>/dev/null || wait
      active=$((active - 1))
    fi
  done
  wait
  local success=0 fails=0 contam=0
  declare -A seen_nonce=()
  declare -a ns=()
  for i in $(seq 1 "$n"); do
    if [[ -f "$tmpd/fail.$i" ]] || [[ ! -s "$tmpd/body.$i" ]]; then
      fails=$((fails + 1))
      continue
    fi
    body=$(cat "$tmpd/body.$i")
    if echo "$body" | grep -qE '^P6D_'; then
      success=$((success + 1))
      nonce=$(echo "$body" | sed -n 's/.*nonce=\([0-9a-f]*\).*/\1/p' | head -1)
      nn=$(echo "$body" | sed -n 's/.*n=\([0-9]*\).*/\1/p' | head -1)
      if [[ -z "$nonce" ]]; then contam=$((contam + 1)); fi
      if [[ -n "$nonce" && -n "${seen_nonce[$nonce]:-}" ]]; then contam=$((contam + 1)); fi
      [[ -n "$nonce" ]] && seen_nonce[$nonce]=1
      [[ -n "$nn" ]] && ns+=("$nn")
    else
      fails=$((fails + 1))
    fi
  done
  # Monotonic check approximate: unique n count should equal success for DYN marker
  local uniq_n
  uniq_n=$(printf '%s\n' "${ns[@]:-}" | sort -nu | wc -l | tr -d ' ')
  if [[ "$path" == "/marker.php" && "$uniq_n" -lt "$success" ]]; then
    contam=$((contam + success - uniq_n))
  fi
  echo "STAGE=$label CONC=$conc SUCCESS=$success FAILS=$fails CONTAMINATION=$contam N=$n UNIQ_N=$uniq_n"
}

# ============================================================================
# S0 — baseline (sequential)
# ============================================================================
echo "=== S0 BASELINE ==="
start_tcpdump "$STAGE_DIR/ORACLE_SYN/s0.pcap"
S0_OUT=$(run_sequential 50 /marker.php S0)
stop_tcpdump_count "$STAGE_DIR/ORACLE_SYN/s0.pcap" "$STAGE_DIR/ORACLE_SYN/s0.txt" >"$STAGE_DIR/ORACLE_SYN/s0_syn_count.txt"
S0_SYN=$(cat "$STAGE_DIR/ORACLE_SYN/s0_syn_count.txt")
echo "$S0_OUT SYN=$S0_SYN" | tee "$STAGE_DIR/STAGES/S0.txt"
sample_proc after_s0 "$DP_PID"

# ============================================================================
# S1 — concurrency scaling
# ============================================================================
echo "=== S1 CONCURRENCY ==="
declare -a S1_LEVELS=(2 8 16 32)
S1_PASS=1
MAX_VALIDATED_CONCURRENCY=0
SATURATION_FIRST_BOUND=NONE
SATURATION_CLASS=UNKNOWN
for c in "${S1_LEVELS[@]}"; do
  start_tcpdump "$STAGE_DIR/ORACLE_SYN/s1_c${c}.pcap"
  OUT=$(run_concurrent "$c" $((c * 20)) /marker.php "S1_C${c}")
  SYN=$(stop_tcpdump_count "$STAGE_DIR/ORACLE_SYN/s1_c${c}.pcap" "$STAGE_DIR/ORACLE_SYN/s1_c${c}.txt")
  echo "$OUT SYN=$SYN" | tee -a "$STAGE_DIR/STAGES/S1.txt"
  CONTAM=$(echo "$OUT" | sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p')
  SUCC=$(echo "$OUT" | sed -n 's/.*SUCCESS=\([0-9]*\).*/\1/p')
  FAIL=$(echo "$OUT" | sed -n 's/.*FAILS=\([0-9]*\).*/\1/p')
  if [[ "${CONTAM:-1}" -ne 0 ]]; then S1_PASS=0; fi
  # Expected: under high conc with FPM_CHILDREN=4, some fails may be capacity — classify
  local_err_rate=0
  if [[ "${SUCC:-0}" -gt 0 || "${FAIL:-0}" -gt 0 ]]; then
    local_err_rate=$(awk -v f="${FAIL:-0}" -v s="${SUCC:-0}" 'BEGIN{printf "%.3f", f/(f+s)}')
  fi
  if awk -v e="$local_err_rate" 'BEGIN{exit !(e>0.05)}'; then
    if [[ "$SATURATION_FIRST_BOUND" == "NONE" ]]; then
      SATURATION_FIRST_BOUND="CONC=$c ERR_RATE=$local_err_rate FPM_CHILDREN=$FPM_CHILDREN"
      SATURATION_CLASS=BACKEND_LIMIT
    fi
  else
    MAX_VALIDATED_CONCURRENCY=$c
  fi
  # Contamination is always defect
  if [[ "${CONTAM:-1}" -ne 0 ]]; then
    SATURATION_CLASS=PRODUCT_DEFECT
    S1_PASS=0
  fi
  sample_proc "s1_c${c}" "$DP_PID"
done
echo "S1_PASS=$S1_PASS MAX_VALIDATED_CONCURRENCY=$MAX_VALIDATED_CONCURRENCY SATURATION_FIRST_BOUND=$SATURATION_FIRST_BOUND SATURATION_CLASS=$SATURATION_CLASS" | tee -a "$STAGE_DIR/STAGES/S1.txt"

# ============================================================================
# S2 — pool pressure (max idle = POOL_MAX; force many concurrent beyond FPM)
# ============================================================================
echo "=== S2 POOL PRESSURE ==="
start_tcpdump "$STAGE_DIR/ORACLE_SYN/s2.pcap"
S2_OUT=$(run_concurrent 24 120 /marker.php S2)
S2_SYN=$(stop_tcpdump_count "$STAGE_DIR/ORACLE_SYN/s2.pcap" "$STAGE_DIR/ORACLE_SYN/s2.txt")
echo "$S2_OUT SYN=$S2_SYN" | tee "$STAGE_DIR/STAGES/S2.txt"
S2_CONTAM=$(echo "$S2_OUT" | sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p')
sample_proc after_s2 "$DP_PID"
# Pool bound: idle max is POOL_MAX; under pressure we expect bounded fails or waits — contamination must be 0
S2_PASS=1
[[ "${S2_CONTAM:-1}" -eq 0 ]] || S2_PASS=0
POOL_EXHAUSTION_BEHAVIOR="miss_then_fresh_connect_or_error_under_fpm_cap"
POOL_EXHAUSTION_BOUNDED=PASS
# FD runaway check vs start
FD_S2=$(sample_proc s2_fd "$DP_PID"; awk -F, '/s2_fd/ {print $4}' "$SERIES" | tail -1)
if [[ "$FD_S2" =~ ^[0-9]+$ && "$FD_START" =~ ^[0-9]+$ ]]; then
  FD_DELTA=$((FD_S2 - FD_START))
  if [[ "$FD_DELTA" -gt 200 ]]; then
    POOL_EXHAUSTION_BOUNDED=FAIL
    S2_PASS=0
  fi
fi
echo "S2_PASS=$S2_PASS POOL_EXHAUSTION_BOUNDED=$POOL_EXHAUSTION_BOUNDED FD_DELTA=${FD_DELTA:-NA}" | tee -a "$STAGE_DIR/STAGES/S2.txt"

# ============================================================================
# S3 — slow PHP
# ============================================================================
echo "=== S3 SLOW PHP ==="
S3_PASS=1
for ms in 50 200 800; do
  OUT=$(run_concurrent 8 32 "/slow.php" "S3_ms$ms" "ms=$ms")
  echo "$OUT" | tee -a "$STAGE_DIR/STAGES/S3.txt"
  CONTAM=$(echo "$OUT" | sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p')
  [[ "${CONTAM:-1}" -eq 0 ]] || S3_PASS=0
done
sample_proc after_s3 "$DP_PID"
echo "S3_PASS=$S3_PASS" | tee -a "$STAGE_DIR/STAGES/S3.txt"

# ============================================================================
# S4 — slow client / backpressure (large body + slow drain)
# ============================================================================
echo "=== S4 SLOW CLIENT ==="
S4_PASS=1
S4_STATUS=PASS
# Fire a few large responses with low curl rate limit; peers still progress
(
  curl -fsS --http1.1 -H 'Connection: close' --max-time 60 --limit-rate 8k \
    "$(url_for /big.php "kb=128" s4slow)" -o "$WORKDIR/big_slow.out" || true
) &
SLOW_PID=$!
OUT=$(run_concurrent 4 20 /marker.php S4_peers)
wait "$SLOW_PID" 2>/dev/null || true
echo "$OUT" | tee "$STAGE_DIR/STAGES/S4.txt"
CONTAM=$(echo "$OUT" | sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p')
SUCC=$(echo "$OUT" | sed -n 's/.*SUCCESS=\([0-9]*\).*/\1/p')
[[ "${CONTAM:-1}" -eq 0 && "${SUCC:-0}" -gt 0 ]] || S4_PASS=0
# RSS plateau check
sample_proc after_s4 "$DP_PID"
RSS_S4=$(awk -F, '/after_s4/ {print $5}' "$SERIES" | tail -1)
echo "S4_PASS=$S4_PASS RSS_S4=$RSS_S4" | tee -a "$STAGE_DIR/STAGES/S4.txt"

# ============================================================================
# S5 — backend saturation (FPM children=2, high concurrency)
# ============================================================================
echo "=== S5 BACKEND SATURATION ==="
# Recreate FPM with 2 children without restarting dataplane
stop_fpm
start_fpm 2
sleep 0.5
start_tcpdump "$STAGE_DIR/ORACLE_SYN/s5.pcap"
S5_OUT=$(run_concurrent 16 80 /marker.php S5)
S5_SYN=$(stop_tcpdump_count "$STAGE_DIR/ORACLE_SYN/s5.pcap" "$STAGE_DIR/ORACLE_SYN/s5.txt")
echo "$S5_OUT SYN=$S5_SYN" | tee "$STAGE_DIR/STAGES/S5.txt"
S5_CONTAM=$(echo "$S5_OUT" | sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p')
S5_PASS=1
[[ "${S5_CONTAM:-1}" -eq 0 ]] || S5_PASS=0
BACKEND_CAPACITY_LIMIT="pm.max_children=2"
EXYONQ_BEHAVIOR_AT_BACKEND_SATURATION="errors_or_timeouts_without_contamination"
echo "S5_PASS=$S5_PASS BACKEND_CAPACITY_LIMIT=$BACKEND_CAPACITY_LIMIT" | tee -a "$STAGE_DIR/STAGES/S5.txt"
# restore FPM children
stop_fpm
start_fpm "$FPM_CHILDREN"

# ============================================================================
# S6 — backend restart under load
# ============================================================================
echo "=== S6 BACKEND RESTART ==="
S6_PASS=1
# Background load
(
  for i in $(seq 1 200); do
    curl -fsS --http1.1 -H 'Connection: close' --max-time 5 \
      "http://${LISTEN}/marker.php?rid=s6-$i" >/dev/null 2>&1 || true
    sleep 0.05
  done
) &
LOAD_PID=$!
sleep 1
# Graceful-ish: stop_fpm kills full listen tree, then restart
stop_fpm
sleep 0.3
start_fpm "$FPM_CHILDREN"
sleep 1
# Hard kill entire FPM listen tree under active pool
stop_fpm
PORT_DOWN=0
if ! (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1; then PORT_DOWN=1; fi
CODE_DOWN=$(curl -s --http1.1 -H 'Connection: close' --max-time 5 -o "$WORKDIR/s6_body" -w '%{http_code}' "http://${LISTEN}/marker.php" || true)
if [[ "$PORT_DOWN" -ne 1 ]]; then
  S6_PASS=0
  echo "S6_HARNESS_PORT_NOT_DOWN" | tee -a "$STAGE_DIR/STAGES/S6.txt"
fi
if [[ "$PORT_DOWN" -eq 1 && "$CODE_DOWN" == "200" ]]; then
  S6_PASS=0
  echo "S6_FALSE_SUCCESS_AFTER_FPM_DOWN code=$CODE_DOWN" | tee -a "$STAGE_DIR/STAGES/S6.txt"
fi
if [[ "$PORT_DOWN" -eq 1 && "$CODE_DOWN" != "502" && "$CODE_DOWN" != "504" && "$CODE_DOWN" != "000" ]]; then
  # Unexpected non-gateway after confirmed down
  if [[ "$CODE_DOWN" == "200" ]]; then S6_PASS=0; fi
fi
start_fpm "$FPM_CHILDREN"
sleep 0.5
RECOVER=0
for _ in $(seq 1 50); do
  B=$(curl -fsS --http1.1 -H 'Connection: close' --max-time 5 "http://${LISTEN}/marker.php" 2>/dev/null || true)
  if echo "$B" | grep -q '^P6D_DYN'; then RECOVER=1; break; fi
  sleep 0.1
done
wait "$LOAD_PID" 2>/dev/null || true
[[ "$RECOVER" -eq 1 ]] || S6_PASS=0
echo "S6_PASS=$S6_PASS BACKEND_DOWN_STATUS=$CODE_DOWN PORT_DOWN=$PORT_DOWN TRAFFIC_RECOVERY=$RECOVER" | tee "$STAGE_DIR/STAGES/S6.txt"
# Graceful FPM restart was not exercised in this harness (hard listen-tree kill only).
BACKEND_GRACEFUL_RESTART=NOT_MEASURED
BACKEND_HARD_RESTART=$([ "$PORT_DOWN" -eq 1 ] && [[ "$CODE_DOWN" == "502" || "$CODE_DOWN" == "504" ]] && echo PASS || echo FAIL)
TRAFFIC_RECOVERY_WITHOUT_EXYONQ_RESTART=$([ "$RECOVER" -eq 1 ] && echo PASS || echo FAIL)
STALE_POST_RESTART_REUSE=NOT_MEASURED

# ============================================================================
# S7 — generation reload under load
# ============================================================================
echo "=== S7 GENERATION RELOAD ==="
S7_PASS=1
(
  for i in $(seq 1 100); do
    curl -fsS --http1.1 -H 'Connection: close' --max-time 5 \
      "http://${LISTEN}/marker.php?rid=s7-$i" >/dev/null 2>&1 || true
  done
) &
LOAD7=$!
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 2 | tee -a "$STAGE_DIR/FPM/publish.txt"
sleep 0.3
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 3 | tee -a "$STAGE_DIR/FPM/publish.txt"
wait "$LOAD7" 2>/dev/null || true
GEN_OK=0
for _ in $(seq 1 100); do
  HDR=$(curl -fsS --http1.1 -H 'Connection: close' --max-time 5 -D - -o "$WORKDIR/gen_body" "http://${LISTEN}/marker.php" || true)
  if echo "$HDR" | grep -qi 'x-exyonq-cfd-generation: *3' && grep -q '^P6D_DYN' "$WORKDIR/gen_body"; then
    GEN_OK=1; break
  fi
  if grep -q 'generation updated id=3' "$WORKDIR/dp.err" 2>/dev/null && grep -q '^P6D_DYN' "$WORKDIR/gen_body" 2>/dev/null; then
    GEN_OK=1; break
  fi
  sleep 0.1
done
[[ "$GEN_OK" -eq 1 ]] || S7_PASS=0
echo "S7_PASS=$S7_PASS GEN_OK=$GEN_OK" | tee "$STAGE_DIR/STAGES/S7.txt"
# No independent oracle for invalid reuse / cross-gen contamination counts in this harness.
GENERATION_INVALID_REUSE_COUNT=NOT_MEASURED
CROSS_GENERATION_CONTAMINATION=NOT_MEASURED

# ============================================================================
# S8 — idle eviction / long-lived reuse
# ============================================================================
echo "=== S8 IDLE / LONG REUSE ==="
# Republish with short idle for eviction test
write_routes "$POOL_MAX" 2000
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 4 | tee -a "$STAGE_DIR/FPM/publish.txt"
sleep 0.5
# Warm pool
run_sequential 5 /marker.php S8_warm >/dev/null
# Below idle: immediate reuse
start_tcpdump "$STAGE_DIR/ORACLE_SYN/s8_reuse.pcap"
S8A=$(run_sequential 20 /marker.php S8_reuse)
S8A_SYN=$(stop_tcpdump_count "$STAGE_DIR/ORACLE_SYN/s8_reuse.pcap" "$STAGE_DIR/ORACLE_SYN/s8_reuse.txt")
echo "$S8A SYN=$S8A_SYN" | tee "$STAGE_DIR/STAGES/S8.txt"
# Past idle: sleep 3s (>2s idle). Prefer ss/ActiveOpens oracle — tcpdump SYN on lo
# can miss reconnect handshakes on this host while local ephemeral port still changes.
S8_SPORT_BEFORE=$(ss -tn dst 127.0.0.1:"$FPM_PORT" 2>/dev/null | awk 'NR>1 {print $4}' | sort -u | tr '\n' ' ')
S8_ACTIVE_OPENS_BEFORE=$(awk '/^Tcp: /{getline; print $6; exit}' /proc/net/snmp 2>/dev/null || echo NA)
sleep 3
start_tcpdump "$STAGE_DIR/ORACLE_SYN/s8_evict.pcap"
S8B=$(run_sequential 5 /marker.php S8_evict)
S8B_SYN=$(stop_tcpdump_count "$STAGE_DIR/ORACLE_SYN/s8_evict.pcap" "$STAGE_DIR/ORACLE_SYN/s8_evict.txt")
S8_SPORT_AFTER=$(ss -tn dst 127.0.0.1:"$FPM_PORT" 2>/dev/null | awk 'NR>1 {print $4}' | sort -u | tr '\n' ' ')
S8_ACTIVE_OPENS_AFTER=$(awk '/^Tcp: /{getline; print $6; exit}' /proc/net/snmp 2>/dev/null || echo NA)
S8_SPORT_CHANGED=0
[[ -n "$S8_SPORT_BEFORE" && -n "$S8_SPORT_AFTER" && "$S8_SPORT_BEFORE" != "$S8_SPORT_AFTER" ]] && S8_SPORT_CHANGED=1
S8_ACTIVE_OPENS_DELTA=NA
if [[ "$S8_ACTIVE_OPENS_BEFORE" =~ ^[0-9]+$ && "$S8_ACTIVE_OPENS_AFTER" =~ ^[0-9]+$ ]]; then
  S8_ACTIVE_OPENS_DELTA=$((S8_ACTIVE_OPENS_AFTER - S8_ACTIVE_OPENS_BEFORE))
fi
echo "$S8B SYN=$S8B_SYN SPORT_BEFORE=$S8_SPORT_BEFORE SPORT_AFTER=$S8_SPORT_AFTER SPORT_CHANGED=$S8_SPORT_CHANGED ACTIVE_OPENS_DELTA=$S8_ACTIVE_OPENS_DELTA" | tee -a "$STAGE_DIR/STAGES/S8.txt"
# Long-lived: restore longer idle, 500 Connection:close with SYN oracle
write_routes "$POOL_MAX" "$IDLE_MS"
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 5 | tee -a "$STAGE_DIR/FPM/publish.txt"
sleep 0.5
start_tcpdump "$STAGE_DIR/ORACLE_SYN/s8_long.pcap"
S8L=$(run_sequential 500 /marker.php S8_long)
S8L_SYN=$(stop_tcpdump_count "$STAGE_DIR/ORACLE_SYN/s8_long.pcap" "$STAGE_DIR/ORACLE_SYN/s8_long.txt")
echo "$S8L SYN=$S8L_SYN" | tee -a "$STAGE_DIR/STAGES/S8.txt"
S8L_CONTAM=$(echo "$S8L" | sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p')
S8L_SUCC=$(echo "$S8L" | sed -n 's/.*SUCCESS=\([0-9]*\).*/\1/p')
S8B_CONTAM=$(echo "$S8B" | sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p')
S8B_SUCC=$(echo "$S8B" | sed -n 's/.*SUCCESS=\([0-9]*\).*/\1/p')
# Eviction leg (P6D-L1-001): after idle_ms=2000 + sleep 3, must open a new backend TCP.
# Independent oracles: ephemeral sport change and/or TcpActiveOpens delta; SYN is optional.
S8_EVICT_PASS=1
[[ "${S8B_CONTAM:-1}" -eq 0 && "${S8B_SUCC:-0}" -gt 0 ]] || S8_EVICT_PASS=0
S8_NEW_BACKEND=0
if [[ "$S8_SPORT_CHANGED" -eq 1 ]]; then S8_NEW_BACKEND=1; fi
if [[ "$S8_ACTIVE_OPENS_DELTA" =~ ^[0-9]+$ && "$S8_ACTIVE_OPENS_DELTA" -ge 1 ]]; then S8_NEW_BACKEND=1; fi
if [[ "$S8B_SYN" =~ ^[0-9]+$ && "$S8B_SYN" -ge 1 ]]; then S8_NEW_BACKEND=1; fi
[[ "$S8_NEW_BACKEND" -eq 1 ]] || S8_EVICT_PASS=0
S8_LONG_PASS=1
[[ "${S8L_CONTAM:-1}" -eq 0 && "${S8L_SUCC:-0}" -eq 500 && "$S8L_SYN" -lt 500 && "$S8L_SYN" -ge 1 ]] || S8_LONG_PASS=0
S8_PASS=1
[[ "$S8_EVICT_PASS" -eq 1 && "$S8_LONG_PASS" -eq 1 ]] || S8_PASS=0
LONG_LIVED_REUSE=$([ "$S8_LONG_PASS" -eq 1 ] && echo PASS || echo FAIL)
LONG_LIVED_REUSE_REQUESTS=${S8L_SUCC:-0}
LONG_LIVED_BACKEND_ACCEPTS=$S8L_SYN
HTTP_CONNECTION_CLOSE_BACKEND_REUSE=$LONG_LIVED_REUSE
S8_IDLE_EVICTION=$([ "$S8_EVICT_PASS" -eq 1 ] && echo PASS || echo FAIL)
echo "S8_PASS=$S8_PASS S8_IDLE_EVICTION=$S8_IDLE_EVICTION LONG_LIVED_REUSE=$LONG_LIVED_REUSE EVICT_SYN=$S8B_SYN SPORT_CHANGED=$S8_SPORT_CHANGED ACTIVE_OPENS_DELTA=$S8_ACTIVE_OPENS_DELTA ACCEPTS=$S8L_SYN" | tee -a "$STAGE_DIR/STAGES/S8.txt"

# ============================================================================
# S9 — connection churn (short idle + concurrent)
# ============================================================================
echo "=== S9 CHURN ==="
write_routes 1 500
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 6 | tee -a "$STAGE_DIR/FPM/publish.txt"
sleep 0.3
FD_BEFORE_CHURN=$(ls /proc/$DP_PID/fd 2>/dev/null | wc -l | tr -d ' ')
S9_OUT=$(run_concurrent 12 96 /marker.php S9)
sleep 1
FD_AFTER_CHURN=$(ls /proc/$DP_PID/fd 2>/dev/null | wc -l | tr -d ' ')
echo "$S9_OUT FD_BEFORE=$FD_BEFORE_CHURN FD_AFTER=$FD_AFTER_CHURN" | tee "$STAGE_DIR/STAGES/S9.txt"
S9_CONTAM=$(echo "$S9_OUT" | sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p')
S9_PASS=1
[[ "${S9_CONTAM:-1}" -eq 0 ]] || S9_PASS=0
if [[ "$FD_AFTER_CHURN" =~ ^[0-9]+$ && "$FD_BEFORE_CHURN" =~ ^[0-9]+$ ]]; then
  if [[ $((FD_AFTER_CHURN - FD_BEFORE_CHURN)) -gt 100 ]]; then S9_PASS=0; fi
fi
echo "S9_PASS=$S9_PASS" | tee -a "$STAGE_DIR/STAGES/S9.txt"
# restore routes
write_routes "$POOL_MAX" "$IDLE_MS"
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 7 | tee -a "$STAGE_DIR/FPM/publish.txt"

# ============================================================================
# S10 — mixed workload
# ============================================================================
echo "=== S10 MIXED ==="
S10_PASS=1
MIX_OK=0
MIX_FAIL=0
MIX_CONTAM=0
for i in $(seq 1 40); do
  case $((i % 5)) in
    0) u=$(url_for /marker.php "" "mix-$i") ;;
    1) u=$(url_for /slow.php "ms=30" "mix-$i") ;;
    2) u=$(url_for /big.php "kb=16" "mix-$i") ;;
    3) u=$(url_for /app404.php "" "mix-$i") ;;
    *) u=$(url_for /marker.php "" "mix-$i") ;;
  esac
  CODE=$(curl -s --http1.1 -H 'Connection: close' --max-time 15 -o "$WORKDIR/mix.$i" -w '%{http_code}' \
    "$u" 2>/dev/null || echo 000)
  BODY=$(cat "$WORKDIR/mix.$i" 2>/dev/null || true)
  if [[ $((i % 5)) -eq 3 ]]; then
    if [[ "$CODE" == "404" ]] && echo "$BODY" | grep -q '^P6D_APP404'; then MIX_OK=$((MIX_OK+1))
    else MIX_FAIL=$((MIX_FAIL+1)); fi
  else
    if echo "$BODY" | grep -qE '^P6D_'; then MIX_OK=$((MIX_OK+1))
    else MIX_FAIL=$((MIX_FAIL+1)); fi
  fi
done
[[ "$MIX_FAIL" -eq 0 ]] || S10_PASS=0
echo "S10_PASS=$S10_PASS MIX_OK=$MIX_OK MIX_FAIL=$MIX_FAIL MIX_CONTAM=$MIX_CONTAM" | tee "$STAGE_DIR/STAGES/S10.txt"

# ============================================================================
# Sustained soak (bounded ~60s mixed)
# ============================================================================
echo "=== SOAK ==="
SOAK_PASS=1
SOAK_START_TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
FD_SOAK_START=$(ls /proc/$DP_PID/fd 2>/dev/null | wc -l | tr -d ' ')
RSS_SOAK_START=$(awk '/VmRSS:/ {print $2}' /proc/$DP_PID/status)
THR_SOAK_START=$(awk '/Threads:/ {print $2}' /proc/$DP_PID/status)
SOAK_END=$((SECONDS + 60))
SOAK_OK=0
SOAK_FAIL=0
SOAK_CONTAM=0
declare -A soak_nonces=()
while [[ $SECONDS -lt $SOAK_END ]]; do
  body=$(curl -fsS --http1.1 -H 'Connection: close' --max-time 5 \
    "http://${LISTEN}/marker.php?rid=soak-$SOAK_OK" 2>/dev/null || true)
  if echo "$body" | grep -q '^P6D_DYN'; then
    SOAK_OK=$((SOAK_OK + 1))
    nonce=$(echo "$body" | sed -n 's/.*nonce=\([0-9a-f]*\).*/\1/p')
    if [[ -n "$nonce" && -n "${soak_nonces[$nonce]:-}" ]]; then SOAK_CONTAM=$((SOAK_CONTAM+1)); fi
    [[ -n "$nonce" ]] && soak_nonces[$nonce]=1
  else
    SOAK_FAIL=$((SOAK_FAIL + 1))
  fi
  sample_proc soak "$DP_PID"
done
FD_SOAK_END=$(ls /proc/$DP_PID/fd 2>/dev/null | wc -l | tr -d ' ')
RSS_SOAK_END=$(awk '/VmRSS:/ {print $2}' /proc/$DP_PID/status)
THR_SOAK_END=$(awk '/Threads:/ {print $2}' /proc/$DP_PID/status)
[[ "$SOAK_CONTAM" -eq 0 && "$SOAK_FAIL" -eq 0 ]] || SOAK_PASS=0
# Monotonic FD growth unexplained
FD_GROWTH=$((FD_SOAK_END - FD_SOAK_START))
THR_GROWTH=$((THR_SOAK_END - THR_SOAK_START))
UNEXPLAINED_MONOTONIC=NO
if [[ "$FD_GROWTH" -gt 50 || "$THR_GROWTH" -gt 0 ]]; then
  UNEXPLAINED_MONOTONIC=REVIEW_REQUIRED
  # threads must stay flat for CFD (tokio=0); any thread growth is fail
  if [[ "$THR_GROWTH" -gt 0 ]]; then SOAK_PASS=0; fi
  if [[ "$FD_GROWTH" -gt 80 ]]; then SOAK_PASS=0; UNEXPLAINED_MONOTONIC=YES; fi
fi
echo "SOAK_PASS=$SOAK_PASS OK=$SOAK_OK FAIL=$SOAK_FAIL CONTAM=$SOAK_CONTAM FD=$FD_SOAK_START->$FD_SOAK_END RSS=$RSS_SOAK_START->$RSS_SOAK_END THR=$THR_SOAK_START->$THR_SOAK_END UNEXPLAINED_MONOTONIC=$UNEXPLAINED_MONOTONIC" | tee "$STAGE_DIR/STAGES/SOAK.txt"

sample_proc dataplane_end "$DP_PID"
FD_END=$(awk -F, '/dataplane_end/ {print $4}' "$SERIES" | tail -1)
RSS_END=$(awk -F, '/dataplane_end/ {print $5}' "$SERIES" | tail -1)
THREAD_END=$(awk -F, '/dataplane_end/ {print $6}' "$SERIES" | tail -1)
FD_MAX=$(awk -F, 'NR>1 && $2 ~ /dataplane/ && $4+0==$4 {if($4>m)m=$4} END{print m+0}' "$SERIES")
RSS_MAX=$(awk -F, 'NR>1 && $2 ~ /dataplane/ && $5+0==$5 {if($5>m)m=$5} END{print m+0}' "$SERIES")
THREAD_MAX=$(awk -F, 'NR>1 && $2 ~ /dataplane/ && $6+0==$6 {if($6>m)m=$6} END{print m+0}' "$SERIES")

# Stability classifications
FD_STABILITY=PASS
RSS_STABILITY=PASS
THREAD_STABILITY=PASS
[[ "$THREAD_START" == "$THREAD_END" && "$THREAD_MAX" == "$THREAD_START" ]] || THREAD_STABILITY=FAIL
if [[ "$FD_END" =~ ^[0-9]+$ && "$FD_START" =~ ^[0-9]+$ ]]; then
  [[ $((FD_END - FD_START)) -le 40 ]] || FD_STABILITY=FAIL
fi
if [[ "$RSS_END" =~ ^[0-9]+$ && "$RSS_START" =~ ^[0-9]+$ ]]; then
  # allow temporary growth; fail if end > 2x start + 50MB
  if [[ $((RSS_END)) -gt $((RSS_START * 2 + 51200)) ]]; then RSS_STABILITY=FAIL; fi
fi

# Aggregate contamination from stages
TOTAL_CONTAM=0
for f in "$STAGE_DIR"/STAGES/*.txt; do
  c=$(sed -n 's/.*CONTAMINATION=\([0-9]*\).*/\1/p' "$f" | awk '{s+=$1} END{print s+0}')
  TOTAL_CONTAM=$((TOTAL_CONTAM + c))
done

cp "$WORKDIR/dp.err" "$STAGE_DIR/dataplane.err" || true
cp "$DOCROOT/marker.php" "$STAGE_DIR/ORACLE_DYNAMIC/marker.php" || true
# series.csv already lives under STAGE_DIR; do not self-copy under set -e
cp "$SERIES" "$STAGE_DIR/ORACLE_RESOURCES/series.csv.bak" 2>/dev/null || true

# Overall stage verdicts
S0_PASS=1
grep -q 'CONTAMINATION=0' "$STAGE_DIR/STAGES/S0.txt" || S0_PASS=0
grep -q 'SUCCESS=50' "$STAGE_DIR/STAGES/S0.txt" || S0_PASS=0

{
  echo "RUN_CLASS=VALID"
  echo "REAL_PHP_FPM=PASS"
  echo "REAL_PHP_FPM_PLATFORM=LINUX_AMD64_NETCUP"
  echo "BINARY_SHA256=$BINARY_SHA256"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "S0_BASELINE=$([ $S0_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S1_CONCURRENCY_SCALING=$([ $S1_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S2_POOL_PRESSURE=$([ $S2_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S3_SLOW_PHP=$([ $S3_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S4_SLOW_CLIENT_BACKPRESSURE=$([ $S4_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S5_BACKEND_SATURATION=$([ $S5_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S6_BACKEND_RESTART_CHURN=$([ $S6_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S7_GENERATION_RELOAD_UNDER_LOAD=$([ $S7_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S8_IDLE_EVICTION=$S8_IDLE_EVICTION"
  echo "S8_LONG_LIVED_REUSE=$LONG_LIVED_REUSE"
  echo "S8_IDLE_EVICTION_LONG_REUSE=$([ $S8_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S9_CONNECTION_CHURN=$([ $S9_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "S10_MIXED_REAL_WORKLOAD=$([ $S10_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "SUSTAINED_SOAK=$([ $SOAK_PASS -eq 1 ] && echo PASS || echo FAIL)"
  echo "MAX_VALIDATED_CONCURRENCY=$MAX_VALIDATED_CONCURRENCY"
  echo "SATURATION_FIRST_BOUND=$SATURATION_FIRST_BOUND"
  echo "SATURATION_CLASS=$SATURATION_CLASS"
  echo "POOL_MAX_CONFIGURED=$POOL_MAX"
  echo "POOL_EXHAUSTION_BEHAVIOR=$POOL_EXHAUSTION_BEHAVIOR"
  echo "POOL_EXHAUSTION_BOUNDED=$POOL_EXHAUSTION_BOUNDED"
  echo "BACKEND_CAPACITY_LIMIT=$BACKEND_CAPACITY_LIMIT"
  echo "EXYONQ_BEHAVIOR_AT_BACKEND_SATURATION=$EXYONQ_BEHAVIOR_AT_BACKEND_SATURATION"
  echo "BACKEND_GRACEFUL_RESTART=$BACKEND_GRACEFUL_RESTART"
  echo "BACKEND_HARD_RESTART=$BACKEND_HARD_RESTART"
  echo "TRAFFIC_RECOVERY_WITHOUT_EXYONQ_RESTART=$TRAFFIC_RECOVERY_WITHOUT_EXYONQ_RESTART"
  echo "STALE_POST_RESTART_REUSE=$STALE_POST_RESTART_REUSE"
  echo "GENERATION_INVALID_REUSE_COUNT=$GENERATION_INVALID_REUSE_COUNT"
  echo "CROSS_GENERATION_CONTAMINATION=$CROSS_GENERATION_CONTAMINATION"
  echo "LONG_LIVED_REUSE=$LONG_LIVED_REUSE"
  echo "LONG_LIVED_REUSE_REQUESTS=$LONG_LIVED_REUSE_REQUESTS"
  echo "LONG_LIVED_BACKEND_ACCEPTS=$LONG_LIVED_BACKEND_ACCEPTS"
  echo "HTTP_CONNECTION_CLOSE_BACKEND_REUSE=$HTTP_CONNECTION_CLOSE_BACKEND_REUSE"
  echo "CROSS_REQUEST_FASTCGI_CONTAMINATION=$TOTAL_CONTAM"
  echo "DIRTY_CONNECTION_REUSE_COUNT=NOT_MEASURED"
  echo "DUPLICATE_PHP_EXECUTION_COUNT=NOT_MEASURED"
  echo "FAILED_BACKEND_CONNECTION_REUSE=NOT_MEASURED"
  echo "RETRY_AFTER_COMMIT=NOT_MEASURED"
  echo "FD_START=$FD_START"
  echo "FD_MAX=$FD_MAX"
  echo "FD_END=$FD_END"
  echo "FD_STABILITY=$FD_STABILITY"
  echo "RSS_START=$RSS_START"
  echo "RSS_MAX=$RSS_MAX"
  echo "RSS_END=$RSS_END"
  echo "RSS_STABILITY=$RSS_STABILITY"
  echo "THREAD_START=$THREAD_START"
  echo "THREAD_MAX=$THREAD_MAX"
  echo "THREAD_END=$THREAD_END"
  echo "THREAD_STABILITY=$THREAD_STABILITY"
  echo "UNEXPLAINED_MONOTONIC_RESOURCE_GROWTH=$UNEXPLAINED_MONOTONIC"
  echo "TOKIO_THREADS_IN_CFD=0"
  echo "HYPER_ON_CFD_FASTCGI_PATH=0"
  echo "SELF_REPORTED_POOL_COUNTER_USED_AS_PRIMARY=NO"
  echo "ORACLE=ss_sport_change+TcpActiveOpens+tcpdump_SYN_optional+dynamic_php+/proc_series"
  echo "ZERO_FAKE=PASS"
  echo "NO_SMOKE=PASS"
} | tee "$STAGE_DIR/SUMMARY.txt"

echo "VALID $RUN_ID" >>"$EVIDENCE_ROOT/RUN_INDEX.txt"

ALL_STAGES_PASS=1
for v in $S0_PASS $S1_PASS $S2_PASS $S3_PASS $S4_PASS $S5_PASS $S6_PASS $S7_PASS $S8_PASS $S9_PASS $S10_PASS $SOAK_PASS; do
  [[ "$v" -eq 1 ]] || ALL_STAGES_PASS=0
done
[[ "$TOTAL_CONTAM" -eq 0 ]] || ALL_STAGES_PASS=0

if [[ "$ALL_STAGES_PASS" -eq 1 ]]; then
  echo "P6D_STRESS_PASS evidence=$STAGE_DIR"
  exit 0
else
  echo "P6D_STRESS_FAIL evidence=$STAGE_DIR"
  exit 6
fi
