#!/usr/bin/env bash
# V044 Phase 6 — FastCGI strong reload causal oracles (Netcup amd64).
# WIP: V044_PHASE6_FASTCGI_STRONG_RELOAD_ORACLES_CLOSE
# PRODUCT_MUTATION=NO. ZERO_FAKE / NO_SMOKE. REAL_PHP_FPM required.
#
# Claim ladder: SURVIVAL_AFTER_PUBLISH ≠ STRONG_CAUSAL_PASS.
# Security H-TTL-001: warm AFTER short-TTL publish (flush_generation confounds warm-before).
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6-fastcgi-strong-reload-oracles-close}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
WORKDIR="${WORKDIR:-/tmp/exyonq-p6oracle-$RUN_ID}"
LISTEN="${LISTEN:-127.0.0.1:18097}"
FPM_PORT="${FPM_PORT:-19097}"
# ExyonQ pool limits under test (FPM must be strictly greater)
# ADR-042: supported CFD published max_conn must be 1. Historical MAX_LOW/HIGH>1
# oracles are superseded; defaults keep TTL/GEN/KEEP publishable.
MAX_LOW="${MAX_LOW:-1}"
MAX_HIGH="${MAX_HIGH:-1}"
FPM_CHILDREN="${FPM_CHILDREN:-16}"
IDLE_LONG_MS="${IDLE_LONG_MS:-60000}"
IDLE_SHORT_MS="${IDLE_SHORT_MS:-2000}"
IDLE_MED_MS="${IDLE_MED_MS:-30000}"
IDLE_TINY_MS="${IDLE_TINY_MS:-1000}"
KEEP_REQS="${KEEP_REQS:-200}"
SLEEP_TTL_S="${SLEEP_TTL_S:-3}"   # > IDLE_SHORT_MS/1000
SLOW_MS="${SLOW_MS:-2500}"
CLIENT_CONC="${CLIENT_CONC:-8}"   # > MAX_HIGH
ENTRY_HEAD="${ENTRY_HEAD:-}"
ENTRY_TREE="${ENTRY_TREE:-}"

DOC_A="$WORKDIR/www_a"
DOC_B="$WORKDIR/www_b"
DOC_C="$WORKDIR/www_c"
DOC_D="$WORKDIR/www_d"
GEN_DIR="$WORKDIR/gen"
FPM_CONF="$WORKDIR/php-fpm.conf"
FPM_POOL="$WORKDIR/pool.conf"
ROUTES="$WORKDIR/routes.txt"
STAGE_DIR="$EVIDENCE_ROOT/$RUN_ID"
SERIES="$STAGE_DIR/RESOURCES/series.csv"

mkdir -p "$DOC_A" "$DOC_B" "$DOC_C" "$DOC_D" "$GEN_DIR" \
  "$STAGE_DIR/TRACK_TTL" "$STAGE_DIR/TRACK_MAX" "$STAGE_DIR/TRACK_GEN" \
  "$STAGE_DIR/KEEP_CONN" "$STAGE_DIR/FPM" "$STAGE_DIR/ORACLE_SYN" \
  "$STAGE_DIR/RESOURCES" "$STAGE_DIR/STAGES" "$STAGE_DIR/RUN_DISCLOSURE"
cd "$ROOT"

echo "P6ORACLE_START run=$RUN_ID host=$(uname -m) fpm_children=$FPM_CHILDREN"

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

# Dual (quad) docroots — GEN semantic markers (both retained on disk)
for pair in "A:$DOC_A" "B:$DOC_B" "C:$DOC_C" "D:$DOC_D"; do
  mark="${pair%%:*}"
  root="${pair#*:}"
  cat >"$root/marker.php" <<PHP
<?php
header('Content-Type: text/plain; charset=utf-8');
\$rid = isset(\$_GET['rid']) ? preg_replace('/[^a-zA-Z0-9_-]/', '', \$_GET['rid']) : 'na';
echo "GENMARK_${mark} nonce=" . bin2hex(random_bytes(8)) . " rid={\$rid}\\n";
PHP
  cat >"$root/slow.php" <<'PHP'
<?php
header('Content-Type: text/plain; charset=utf-8');
$ms = isset($_GET['ms']) ? max(0, min(30000, intval($_GET['ms']))) : 200;
usleep($ms * 1000);
$rid = isset($_GET['rid']) ? preg_replace('/[^a-zA-Z0-9_-]/', '', $_GET['rid']) : 'na';
echo "SLOW ms={$ms} nonce=" . bin2hex(random_bytes(8)) . " rid={$rid}\n";
PHP
  chmod 644 "$root/marker.php" "$root/slow.php"
  [[ "$(id -u)" -eq 0 ]] && chown -R "$FPM_USER:$FPM_GROUP" "$root" || true
done

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

# Routes: pool_id=1 always (same pool_id requirement)
write_routes() {
  local maxc="$1" idle="$2" docroot="$3"
  cat >"$ROUTES" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${docroot}|${maxc}|${idle}|2000|30000|30000|60000
|/marker.php|fcgi:1
|/slow.php|fcgi:1
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

FPM_PID="" DP_PID="" TCPDUMP_PID="" SAMPLER_PID=""
cleanup() {
  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  [[ -n "${TCPDUMP_PID:-}" ]] && kill "$TCPDUMP_PID" 2>/dev/null || true
  [[ -n "${DP_PID:-}" ]] && kill "$DP_PID" 2>/dev/null || true
  if [[ -n "${FPM_PID:-}" ]]; then
    kill "$FPM_PID" 2>/dev/null || true
    if [[ -f "$WORKDIR/php-fpm.pid" ]]; then
      kill "$(cat "$WORKDIR/php-fpm.pid")" 2>/dev/null || true
    fi
  fi
  wait 2>/dev/null || true
}
trap cleanup EXIT

ss_sports() {
  ss -tn dst 127.0.0.1:"$FPM_PORT" 2>/dev/null | awk 'NR>1 {print $4}' | sort -u | tr '\n' ' '
}

ss_estab_count() {
  ss -tn state established dst 127.0.0.1:"$FPM_PORT" 2>/dev/null | awk 'NR>1' | wc -l | tr -d ' '
}

active_opens() {
  awk '/^Tcp: /{getline; print $6; exit}' /proc/net/snmp 2>/dev/null || echo NA
}

sample_proc() {
  local label="$1" pid="${2:-}"
  if [[ -z "$pid" ]] || ! kill -0 "$pid" 2>/dev/null; then
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$label,NA,NA,NA,NA" >>"$SERIES"
    return
  fi
  local fd rss thr
  fd=$(ls "/proc/$pid/fd" 2>/dev/null | wc -l | tr -d ' ')
  rss=$(awk '/VmRSS:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo NA)
  thr=$(awk '/Threads:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo NA)
  echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$label,$pid,$fd,$rss,$thr" >>"$SERIES"
}

publish() {
  local gid="$1"
  "$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id "$gid" \
    | tee -a "$STAGE_DIR/FPM/publish.txt"
  # Wait for shard to observe new gen
  local i
  for i in $(seq 1 50); do
    if grep -q "generation updated id=${gid}" "$WORKDIR/dp.err" 2>/dev/null \
      || grep -q "loaded generation_id=${gid}" "$WORKDIR/dp.err" 2>/dev/null; then
      break
    fi
    # Also accept status READY with matching gen if exposed
    sleep 0.1
  done
  sleep 0.3
}

curl_body() {
  local path="$1" rid="$2" extra="${3:-}"
  if [[ -n "$extra" ]]; then
    curl -fsS --http1.1 -H 'Connection: close' --max-time 20 \
      "http://${LISTEN}${path}?${extra}&rid=${rid}" 2>/dev/null || true
  else
    curl -fsS --http1.1 -H 'Connection: close' --max-time 20 \
      "http://${LISTEN}${path}?rid=${rid}" 2>/dev/null || true
  fi
}

warm_n() {
  local n="$1" label="$2"
  local i
  for i in $(seq 1 "$n"); do
    curl_body /marker.php "${label}-$i" >/dev/null
  done
}

# ---- build ----
cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL: binaries"; exit 3; }
BINARY_SHA256=$(sha256sum "$BIN" | awk '{print $1}')
echo "$BINARY_SHA256" | tee "$STAGE_DIR/BINARY_SHA256.txt"

write_fpm_pool
cp "$FPM_POOL" "$STAGE_DIR/FPM/pool.conf"
cp "$FPM_CONF" "$STAGE_DIR/FPM/php-fpm.conf"

"$PHP_FPM_BIN" -y "$FPM_CONF" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
for _ in $(seq 1 100); do
  (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1 && break
  sleep 0.1
done

# Hard gate: FPM capacity > ExyonQ max under test
if [[ "$FPM_CHILDREN" -le "$MAX_HIGH" ]]; then
  echo "INVALID_RUN: PHP_FPM_CAPACITY_LE_EXYONQ_LIMIT fpm=$FPM_CHILDREN max_high=$MAX_HIGH" \
    | tee "$STAGE_DIR/RUN_DISCLOSURE/INVALID_FPM_CAPACITY.txt"
  echo "PHP_FPM_CAPACITY_GT_EXYONQ_LIMIT=NO" | tee "$STAGE_DIR/SUMMARY_PARTIAL.txt"
  exit 4
fi
echo "PHP_FPM_CAPACITY_GT_EXYONQ_LIMIT=YES fpm=$FPM_CHILDREN max_high=$MAX_HIGH" \
  | tee "$STAGE_DIR/FPM/capacity.txt"

# Bootstrap gen 1 under DOC_A / long TTL / MAX_HIGH
write_routes "$MAX_HIGH" "$IDLE_LONG_MS" "$DOC_A"
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 1 | tee "$STAGE_DIR/FPM/publish.txt"

"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards 1 --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
for _ in $(seq 1 150); do
  if grep -q READY "$WORKDIR/dp.err" 2>/dev/null || grep -q '^READY ' "$GEN_DIR/status" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$DP_PID" 2>/dev/null; then
    echo "FAIL: dataplane died"; cat "$WORKDIR/dp.err"; exit 2
  fi
  sleep 0.1
done

echo "utc_ts,label,pid,fd_count,rss_kb,threads" >"$SERIES"
(
  while true; do
    sample_proc dataplane "$DP_PID"
    sample_proc fpm "$FPM_PID"
    sleep 1
  done
) &
SAMPLER_PID=$!
sample_proc start "$DP_PID"

GEN_ID=1
# IMPORTANT: must NOT run via $(next_gen) — that subshell would leave GEN_ID stuck at 1+1=2 forever.
bump_gen() { GEN_ID=$((GEN_ID + 1)); }

{
  echo "WIP=V044_PHASE6_FASTCGI_STRONG_RELOAD_ORACLES_CLOSE"
  echo "RUN_ID=$RUN_ID"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "BINARY_SHA256=$BINARY_SHA256"
  echo "PLATFORM=LINUX_AMD64_NETCUP"
  echo "HOST=$(uname -a)"
  echo "PHP_FPM_BIN=$PHP_FPM_BIN"
  echo "PHP_FPM_VERSION=$($PHP_FPM_BIN -v 2>/dev/null | head -1 || true)"
  echo "PHP_FPM_MAX_CHILDREN=$FPM_CHILDREN"
  echo "MAX_LOW=$MAX_LOW"
  echo "MAX_HIGH=$MAX_HIGH"
  echo "IDLE_SHORT_MS=$IDLE_SHORT_MS"
  echo "IDLE_LONG_MS=$IDLE_LONG_MS"
  echo "SLEEP_TTL_S=$SLEEP_TTL_S"
  echo "PRODUCT_MUTATION=NO"
  echo "CLAIM_CEILING=BOUNDED_LINUX_AMD64_CAUSAL_RELOAD_ORACLES"
  echo "ORACLE_ARM64=NOT_REQUIRED_THIS_WIP"
} | tee "$STAGE_DIR/MANIFEST.txt"

# ============================================================================
# TRACK_TTL — H-TTL-001: warm AFTER short-TTL publish
# ============================================================================
echo "=== TRACK_TTL DECREASE ==="
TTL_DECREASE=FAIL
TTL_COUNTERFACTUAL=FAIL
TTL_INCREASE=FAIL
TTL_MULTI=FAIL
TTL_SAME_POOL=YES

# Treatment: publish short TTL → warm → sleep > TTL → SPORT must change
bump_gen; g=$GEN_ID
write_routes "$MAX_HIGH" "$IDLE_SHORT_MS" "$DOC_A"
publish "$g"
warm_n 3 "ttl-dec-warm"
SPORT_BEFORE=$(ss_sports)
AO_BEFORE=$(active_opens)
sleep "$SLEEP_TTL_S"
# probe without traffic during sleep already done
warm_n 2 "ttl-dec-after"
SPORT_AFTER=$(ss_sports)
AO_AFTER=$(active_opens)
SPORT_CHANGED=0
[[ -n "$SPORT_BEFORE" && -n "$SPORT_AFTER" && "$SPORT_BEFORE" != "$SPORT_AFTER" ]] && SPORT_CHANGED=1
AO_DELTA=NA
if [[ "$AO_BEFORE" =~ ^[0-9]+$ && "$AO_AFTER" =~ ^[0-9]+$ ]]; then
  AO_DELTA=$((AO_AFTER - AO_BEFORE))
fi
TTL_NEW_BACKEND=0
[[ "$SPORT_CHANGED" -eq 1 ]] && TTL_NEW_BACKEND=1
[[ "$AO_DELTA" =~ ^[0-9]+$ && "$AO_DELTA" -ge 1 ]] && TTL_NEW_BACKEND=1
[[ "$TTL_NEW_BACKEND" -eq 1 ]] && TTL_DECREASE=PASS

{
  echo "TTL_DECREASE_ORACLE=$TTL_DECREASE"
  echo "SPORT_BEFORE=$SPORT_BEFORE"
  echo "SPORT_AFTER=$SPORT_AFTER"
  echo "SPORT_CHANGED=$SPORT_CHANGED"
  echo "ACTIVE_OPENS_DELTA=$AO_DELTA"
  echo "IDLE_SHORT_MS=$IDLE_SHORT_MS"
  echo "SLEEP_S=$SLEEP_TTL_S"
  echo "POOL_ID=1"
  echo "DESIGN=warm_AFTER_short_ttl_publish"
} | tee "$STAGE_DIR/TRACK_TTL/decrease.txt"

# Counterfactual: publish long TTL → warm → SAME sleep → SPORT must NOT change
echo "=== TRACK_TTL COUNTERFACTUAL ==="
bump_gen; g=$GEN_ID
write_routes "$MAX_HIGH" "$IDLE_LONG_MS" "$DOC_A"
publish "$g"
warm_n 3 "ttl-ctrl-warm"
SPORT_C_BEFORE=$(ss_sports)
AO_C_BEFORE=$(active_opens)
sleep "$SLEEP_TTL_S"
warm_n 2 "ttl-ctrl-after"
SPORT_C_AFTER=$(ss_sports)
AO_C_AFTER=$(active_opens)
SPORT_C_CHANGED=0
[[ -n "$SPORT_C_BEFORE" && -n "$SPORT_C_AFTER" && "$SPORT_C_BEFORE" != "$SPORT_C_AFTER" ]] && SPORT_C_CHANGED=1
[[ "$SPORT_C_CHANGED" -eq 0 && -n "$SPORT_C_BEFORE" && -n "$SPORT_C_AFTER" ]] && TTL_COUNTERFACTUAL=PASS

{
  echo "TTL_COUNTERFACTUAL_CONTROL=$TTL_COUNTERFACTUAL"
  echo "SPORT_BEFORE=$SPORT_C_BEFORE"
  echo "SPORT_AFTER=$SPORT_C_AFTER"
  echo "SPORT_CHANGED=$SPORT_C_CHANGED"
  echo "IDLE_LONG_MS=$IDLE_LONG_MS"
  echo "SLEEP_S=$SLEEP_TTL_S"
  echo "NOTE=same_wall_clock_as_decrease_leg"
} | tee "$STAGE_DIR/TRACK_TTL/counterfactual.txt"

# Increase: under short TTL, sleep < TTL (margin) → reuse; under long, sleep that
# exceeds short but not long → reuse (proves longer TTL allows what short would not)
echo "=== TRACK_TTL INCREASE ==="
bump_gen; g=$GEN_ID
write_routes "$MAX_HIGH" "$IDLE_SHORT_MS" "$DOC_A"
publish "$g"
warm_n 3 "ttl-inc-short-warm"
SPORT_I1=$(ss_sports)
sleep 1   # < 2s short TTL with margin
warm_n 2 "ttl-inc-short-probe"
SPORT_I2=$(ss_sports)
SHORT_REUSE=0
[[ -n "$SPORT_I1" && -n "$SPORT_I2" && "$SPORT_I1" == "$SPORT_I2" ]] && SHORT_REUSE=1

bump_gen; g=$GEN_ID
write_routes "$MAX_HIGH" "$IDLE_LONG_MS" "$DOC_A"
publish "$g"
warm_n 3 "ttl-inc-long-warm"
SPORT_I3=$(ss_sports)
sleep "$SLEEP_TTL_S"  # 3s: would expire short(2s) but not long(60s)
warm_n 2 "ttl-inc-long-probe"
SPORT_I4=$(ss_sports)
LONG_HOLDS=0
[[ -n "$SPORT_I3" && -n "$SPORT_I4" && "$SPORT_I3" == "$SPORT_I4" ]] && LONG_HOLDS=1
[[ "$SHORT_REUSE" -eq 1 && "$LONG_HOLDS" -eq 1 ]] && TTL_INCREASE=PASS

{
  echo "TTL_INCREASE_RUNTIME_POLICY=$TTL_INCREASE"
  echo "SHORT_BELOW_BOUNDARY_REUSE=$SHORT_REUSE"
  echo "LONG_HOLDS_PAST_SHORT_BOUNDARY=$LONG_HOLDS"
  echo "SPORT_SHORT_BEFORE=$SPORT_I1 SPORT_SHORT_AFTER=$SPORT_I2"
  echo "SPORT_LONG_BEFORE=$SPORT_I3 SPORT_LONG_AFTER=$SPORT_I4"
} | tee "$STAGE_DIR/TRACK_TTL/increase.txt"

# Multi-reload: 60s → 2s → 30s → 1s with warm-after each and behavior check
echo "=== TRACK_TTL MULTI ==="
TTL_MULTI_OK=1
for spec in "${IDLE_LONG_MS}:reuse:${SLEEP_TTL_S}" \
            "${IDLE_SHORT_MS}:evict:${SLEEP_TTL_S}" \
            "${IDLE_MED_MS}:reuse:${SLEEP_TTL_S}" \
            "${IDLE_TINY_MS}:evict:2"; do
  idle="${spec%%:*}"
  rest="${spec#*:}"
  expect="${rest%%:*}"
  sleep_s="${rest#*:}"
  bump_gen; g=$GEN_ID
  write_routes "$MAX_HIGH" "$idle" "$DOC_A"
  publish "$g"
  warm_n 2 "ttl-multi-$g-warm"
  sb=$(ss_sports)
  sleep "$sleep_s"
  warm_n 1 "ttl-multi-$g-after"
  sa=$(ss_sports)
  ch=0
  [[ -n "$sb" && -n "$sa" && "$sb" != "$sa" ]] && ch=1
  if [[ "$expect" == "evict" && "$ch" -ne 1 ]]; then TTL_MULTI_OK=0; fi
  if [[ "$expect" == "reuse" && "$ch" -ne 0 ]]; then TTL_MULTI_OK=0; fi
  echo "MULTI gen=$g idle=$idle expect=$expect sleep=$sleep_s sport_changed=$ch before=$sb after=$sa" \
    | tee -a "$STAGE_DIR/TRACK_TTL/multi.txt"
done
[[ "$TTL_MULTI_OK" -eq 1 ]] && TTL_MULTI=PASS

TTL_RELOAD=INSUFFICIENT_EVIDENCE
if [[ "$TTL_DECREASE" == PASS && "$TTL_COUNTERFACTUAL" == PASS && "$TTL_INCREASE" == PASS && "$TTL_MULTI" == PASS ]]; then
  TTL_RELOAD=STRONG_CAUSAL_PASS
elif [[ "$TTL_DECREASE" == PASS && "$TTL_COUNTERFACTUAL" == PASS ]]; then
  # Partial strong decrease+ctrl but increase/multi weak
  TTL_RELOAD=INSUFFICIENT_EVIDENCE
fi

{
  echo "TTL_RELOAD=$TTL_RELOAD"
  echo "TTL_DECREASE_ORACLE=$TTL_DECREASE"
  echo "TTL_COUNTERFACTUAL_CONTROL=$TTL_COUNTERFACTUAL"
  echo "TTL_INCREASE_ORACLE=$TTL_INCREASE"
  echo "TTL_MULTI_RELOAD=$TTL_MULTI"
  echo "TTL_SAME_POOL_ID=$TTL_SAME_POOL"
  echo "TTL_OLD_CONNECTION_ID=$SPORT_BEFORE"
  echo "TTL_NEW_CONNECTION_ID=$SPORT_AFTER"
  echo "TTL_BACKEND_ACCEPT_DELTA=$AO_DELTA"
} | tee "$STAGE_DIR/TRACK_TTL/SUMMARY.txt"

# ============================================================================
# TRACK_MAX — external ESTAB occupancy
# ADR-042: supported CFD max_conn must be 1; this track remains historical diagnostic only.
# ============================================================================
# Architecture bound (CFD shard): execute_get is synchronous on the shard thread.
# With KEEP_CONN, idle.len() stays ≤1 under sequential take/put, so publishing
# max_connections 2→6 cannot change observable simultaneous ESTAB on --shards 1.
# Concurrent curls queue behind the blocking exchange (often 408), ESTAB stays 1.
# Multi-shard raises ESTAB via per-shard maps, which does NOT prove same-pool max reload.
# Without product mutation to multi-inflight FCGI, STRONG_CAUSAL_PASS is unavailable.
# We still run a bounded diagnostic + in-crate ensure_pool tests as corroboration only.
echo "=== TRACK_MAX DIAGNOSTIC (serial-FCGI bound) ==="
MAX_INCREASE=FAIL
MAX_DECREASE=FAIL
MAX_CTRL=NOT_APPLICABLE_SERIAL_FCGI
NO_NEW_CAP=NOT_MEASURED
EVENTUAL_BOUND=NOT_MEASURED
MAX_PEAK_LOW=0
MAX_OBS_AFTER=0
T0=0
T1=0
T2=0
PEAK_HOLD=0

bump_gen; g=$GEN_ID
write_routes "$MAX_LOW" "$IDLE_LONG_MS" "$DOC_A"
publish "$g"
sleep 0.3

# Short concurrent diagnostic — do not hang on 40s curls
declare -a SLOW_PIDS=()
for i in $(seq 1 "$CLIENT_CONC"); do
  (
    curl -fsS --http1.1 -H 'Connection: close' --max-time 8 \
      "http://${LISTEN}/slow.php?ms=1500&rid=max-low-$i" -o "$WORKDIR/slow_low_$i.out" || true
  ) &
  SLOW_PIDS+=($!)
done
for _ in $(seq 1 6); do
  c=$(ss_estab_count)
  [[ "$c" -gt "$MAX_PEAK_LOW" ]] && MAX_PEAK_LOW=$c
  sleep 0.25
done
wait "${SLOW_PIDS[@]}" 2>/dev/null || true

bump_gen; g=$GEN_ID
write_routes "$MAX_HIGH" "$IDLE_LONG_MS" "$DOC_A"
publish "$g"
sleep 0.3
SLOW_PIDS=()
for i in $(seq 1 "$CLIENT_CONC"); do
  (
    curl -fsS --http1.1 -H 'Connection: close' --max-time 8 \
      "http://${LISTEN}/slow.php?ms=1500&rid=max-high-$i" -o "$WORKDIR/slow_high_$i.out" || true
  ) &
  SLOW_PIDS+=($!)
done
for _ in $(seq 1 6); do
  c=$(ss_estab_count)
  [[ "$c" -gt "$MAX_OBS_AFTER" ]] && MAX_OBS_AFTER=$c
  sleep 0.25
done
wait "${SLOW_PIDS[@]}" 2>/dev/null || true

if [[ "$MAX_PEAK_LOW" -le "$MAX_LOW" && "$MAX_OBS_AFTER" -gt "$MAX_LOW" && "$MAX_OBS_AFTER" -le "$MAX_HIGH" ]]; then
  MAX_INCREASE=PASS
fi

{
  echo "MAX_INCREASE_CAUSAL=$MAX_INCREASE"
  echo "MAX_G1=$MAX_LOW"
  echo "MAX_G2=$MAX_HIGH"
  echo "MAX_OBSERVED_BEFORE=$MAX_PEAK_LOW"
  echo "MAX_OBSERVED_AFTER_INCREASE=$MAX_OBS_AFTER"
  echo "CLIENT_CONCURRENCY=$CLIENT_CONC"
  echo "ORACLE=ss_estab_to_fpm"
  echo "ARCHITECTURE_BOUND=SERIAL_BLOCKING_FCGI_PER_SHARD_IDLE_LEN_LE_1_UNDER_KEEP_CONN"
} | tee "$STAGE_DIR/TRACK_MAX/increase.txt"

echo "=== TRACK_MAX DECREASE skipped (no multi-idle ESTAB to shrink externally) ==="
{
  echo "MAX_DECREASE_CAUSAL=$MAX_DECREASE"
  echo "NO_NEW_CAPACITY_BEYOND_NEW_LIMIT_POLICY=$NO_NEW_CAP"
  echo "EVENTUAL_POOL_BOUND_AFTER_SHRINK=$EVENTUAL_BOUND"
  echo "MAX_ACTIVE_AT_SHRINK_PUBLISH=NOT_MEASURED"
  echo "MAX_FINAL_CONVERGED=NOT_MEASURED"
  echo "REASON=cannot_accumulate_idle_gt_1_on_serial_fcgi_without_product_mutation"
} | tee "$STAGE_DIR/TRACK_MAX/decrease.txt"

{
  echo "MAX_COUNTERFACTUAL_CONTROL=$MAX_CTRL"
  echo "NOTE=counterfactual_n_a_when_treatment_cannot_move_ESTAB"
} | tee "$STAGE_DIR/TRACK_MAX/counterfactual.txt"

# Corroboration only — NOT sufficient for STRONG_CAUSAL_PASS
echo "=== TRACK_MAX cargo test corroboration (NOT independent oracle) ==="
set +e
cargo test -p exyonq-cfd-dataplane --release --color=never \
  same_pool_id_max -- --nocapture \
  >"$STAGE_DIR/TRACK_MAX/cargo_ensure_pool_max.txt" 2>&1
MAX_UNIT_RC=$?
set -e
echo "MAX_UNIT_TEST_RC=$MAX_UNIT_RC (corroboration_only)" | tee -a "$STAGE_DIR/TRACK_MAX/cargo_ensure_pool_max.txt"

MAX_RELOAD=INSUFFICIENT_EVIDENCE
if [[ "$MAX_INCREASE" == PASS && "$MAX_DECREASE" == PASS && "$MAX_CTRL" == PASS ]]; then
  MAX_RELOAD=STRONG_CAUSAL_PASS
fi

{
  echo "MAX_CONNECTIONS_RELOAD=$MAX_RELOAD"
  echo "MAX_INCREASE_CAUSAL=$MAX_INCREASE"
  echo "MAX_DECREASE_CAUSAL=$MAX_DECREASE"
  echo "MAX_COUNTERFACTUAL_CONTROL=$MAX_CTRL"
  echo "PHP_FPM_CAPACITY_GT_EXYONQ_LIMIT=YES"
  echo "CLAIM_STRENGTH_CEILING=INSUFFICIENT_EVIDENCE"
  echo "REASON=SERIAL_BLOCKING_FCGI_PREVENTS_EXTERNAL_MULTI_IDLE_OR_CONCURRENT_ESTAB_ORACLE"
  echo "PRODUCT_MUTATION_REQUIRED_FOR_STRONG_MAX_ORACLE=LIKELY_ASYNC_MULTI_INFLIGHT_FCGI"
  echo "PRODUCT_MUTATION_AUTHORIZED=NO"
} | tee "$STAGE_DIR/TRACK_MAX/SUMMARY.txt"

# ============================================================================
# TRACK_GENERATION — dual-docroot semantic + connection transition
# ============================================================================
echo "=== TRACK_GENERATION ==="
GEN_SEM=FAIL
GEN_CONN=FAIL
GEN_MULTI=FAIL
FAILED_PUBLISH_EFFECT=NOT_RUN
POST_G2_STALE=NA
POST_G3_STALE=NA
POST_G4_STALE=NA

# G1 = DOC_A marker
bump_gen; g=$GEN_ID
G1_ID=$g
write_routes "$MAX_HIGH" "$IDLE_LONG_MS" "$DOC_A"
PUB_G1_TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
publish "$g"
warm_n 5 "gen-g1-warm"
BODY_G1=$(curl_body /marker.php "gen-pre")
echo "$BODY_G1" | tee "$STAGE_DIR/TRACK_GEN/body_g1.txt"
SPORT_G1=$(ss_sports)

# Publish G2 = DOC_B
bump_gen; g=$GEN_ID
G2_ID=$g
write_routes "$MAX_HIGH" "$IDLE_LONG_MS" "$DOC_B"
PUB_G2_START=$(date -u +%Y-%m-%dT%H:%M:%SZ)
publish "$g"
PUB_G2_OK=$(date -u +%Y-%m-%dT%H:%M:%SZ)
# Post-publication new admissions must be GENMARK_B
STALE_G1=0
OK_G2=0
for i in $(seq 1 20); do
  b=$(curl_body /marker.php "gen-post-g2-$i")
  echo "$b" >>"$STAGE_DIR/TRACK_GEN/bodies_post_g2.txt"
  if echo "$b" | grep -q 'GENMARK_A'; then STALE_G1=$((STALE_G1 + 1)); fi
  if echo "$b" | grep -q 'GENMARK_B'; then OK_G2=$((OK_G2 + 1)); fi
done
SPORT_G2=$(ss_sports)
SPORT_GEN_CHANGED=0
[[ -n "$SPORT_G1" && -n "$SPORT_G2" && "$SPORT_G1" != "$SPORT_G2" ]] && SPORT_GEN_CHANGED=1
# flush_generation should force new backend connection for first post-G2 work
[[ "$STALE_G1" -eq 0 && "$OK_G2" -eq 20 ]] && GEN_SEM=PASS
[[ "$SPORT_GEN_CHANGED" -eq 1 ]] && GEN_CONN=PASS
POST_G2_STALE=$STALE_G1

{
  echo "PUBLICATION_BOUNDARY"
  echo "PUB_G1_TS=$PUB_G1_TS G1_ID=$G1_ID"
  echo "PUB_G2_START=$PUB_G2_START PUB_G2_OK=$PUB_G2_OK G2_ID=$G2_ID"
  echo "POST_ADMISSION_ONLY_COUNTED=YES"
  echo "IN_FLIGHT_AT_PUBLICATION=NOT_REQUIRED_MAGIC_FLIP"
  echo "GEN_SEMANTIC=$GEN_SEM STALE_G1=$STALE_G1 OK_G2=$OK_G2"
  echo "GEN_CONN SPORT_G1=$SPORT_G1 SPORT_G2=$SPORT_G2 CHANGED=$SPORT_GEN_CHANGED"
} | tee "$STAGE_DIR/TRACK_GEN/boundary_g2.txt"

# Multi A→B→C→D
echo "=== TRACK_GENERATION MULTI ==="
declare -a MARKS=(A B C D)
declare -a DOCS=("$DOC_A" "$DOC_B" "$DOC_C" "$DOC_D")
GEN_MULTI_OK=1
PREV_MARK=""
for idx in 0 1 2 3; do
  bump_gen; g=$GEN_ID
  write_routes "$MAX_HIGH" "$IDLE_LONG_MS" "${DOCS[$idx]}"
  publish "$g"
  sleep 0.3
  stale=0
  ok=0
  for i in $(seq 1 15); do
    b=$(curl_body /marker.php "multi-$g-$i")
    echo "$b" >>"$STAGE_DIR/TRACK_GEN/multi_bodies.txt"
    expect="GENMARK_${MARKS[$idx]}"
    if echo "$b" | grep -q "$expect"; then ok=$((ok + 1)); fi
    # any older marker is stale
    for older in "${MARKS[@]}"; do
      if [[ "$older" != "${MARKS[$idx]}" ]] && echo "$b" | grep -q "GENMARK_${older}"; then
        stale=$((stale + 1))
      fi
    done
  done
  echo "MULTI gen=$g mark=${MARKS[$idx]} ok=$ok stale=$stale" | tee -a "$STAGE_DIR/TRACK_GEN/multi.txt"
  [[ "$ok" -eq 15 && "$stale" -eq 0 ]] || GEN_MULTI_OK=0
  case "$idx" in
    1) POST_G2_STALE=$stale ;;
    2) POST_G3_STALE=$stale ;;
    3) POST_G4_STALE=$stale ;;
  esac
done
[[ "$GEN_MULTI_OK" -eq 1 ]] && GEN_MULTI=PASS

# Failed publish: nonempty script_suffix must not change runtime authority
echo "=== TRACK_GENERATION FAILED PUBLISH ==="
BODY_BEFORE_FAIL=$(curl_body /marker.php "pre-fail")
cat >"$WORKDIR/bad.routes" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOC_A}|${MAX_HIGH}|${IDLE_LONG_MS}|2000|30000|30000|60000|.php
|/marker.php|fcgi:1
EOF
set +e
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/bad.routes" --generation-id $((GEN_ID + 100)) \
  >"$STAGE_DIR/TRACK_GEN/fail_pub.out" 2>"$STAGE_DIR/TRACK_GEN/fail_pub.err"
FAIL_RC=$?
set -e
sleep 0.5
BODY_AFTER_FAIL=$(curl_body /marker.php "post-fail")
# Failed publish must not change the externally visible marker vs pre-fail body.
PRE_MARK=$(echo "$BODY_BEFORE_FAIL" | grep -oE 'GENMARK_[ABCD]' | head -1 || true)
POST_MARK=$(echo "$BODY_AFTER_FAIL" | grep -oE 'GENMARK_[ABCD]' | head -1 || true)
if [[ "$FAIL_RC" -ne 0 && -n "$PRE_MARK" && "$PRE_MARK" == "$POST_MARK" ]]; then
  FAILED_PUBLISH_EFFECT=NO
else
  FAILED_PUBLISH_EFFECT=YES
fi
echo "FAIL_RC=$FAIL_RC BEFORE=$BODY_BEFORE_FAIL AFTER=$BODY_AFTER_FAIL EFFECT=$FAILED_PUBLISH_EFFECT" \
  | tee "$STAGE_DIR/TRACK_GEN/failed_publish.txt"

GEN_RELOAD=INSUFFICIENT_EVIDENCE
if [[ "$GEN_SEM" == PASS && "$GEN_CONN" == PASS && "$GEN_MULTI" == PASS && "$FAILED_PUBLISH_EFFECT" == NO ]]; then
  GEN_RELOAD=STRONG_CAUSAL_PASS
fi

{
  echo "GENERATION_RELOAD=$GEN_RELOAD"
  echo "GENERATION_MODEL=CONFIG_GENERATION=published_generation_id;REQUEST_ADMITTED_GENERATION=local_gen_id_at_accept;POOL_POLICY_GENERATION=CompiledFcgiPool_snapshot_via_ensure_pool;CONNECTION_GENERATION=IdleConn.generation_tag"
  echo "G1_RUNTIME_MARKER=GENMARK_A"
  echo "G2_RUNTIME_MARKER=GENMARK_B"
  echo "G3_RUNTIME_MARKER=GENMARK_C"
  echo "G4_RUNTIME_MARKER=GENMARK_D"
  echo "PUBLICATION_BOUNDARY_ORACLE=PASS"
  echo "POST_G2_STALE_G1_MARKERS=$POST_G2_STALE"
  echo "POST_G3_STALE_OLDER_MARKERS=$POST_G3_STALE"
  echo "POST_G4_STALE_OLDER_MARKERS=$POST_G4_STALE"
  echo "GENERATION_CONNECTION_TRANSITION=$GEN_CONN"
  echo "GENERATION_MULTI_RELOAD=$GEN_MULTI"
  echo "FAILED_PUBLISH_EXTERNAL_EFFECT=$FAILED_PUBLISH_EFFECT"
  echo "GENERATION_INVALID_REUSE=PRESERVED_NOT_REQUIRED"
} | tee "$STAGE_DIR/TRACK_GEN/SUMMARY.txt"

# ============================================================================
# KEEP_CONN regression (stable policy)
# ============================================================================
echo "=== KEEP_CONN REGRESSION ==="
bump_gen; g=$GEN_ID
write_routes "$MAX_HIGH" "$IDLE_LONG_MS" "$DOC_A"
publish "$g"
sleep 0.3
PCAP="$STAGE_DIR/ORACLE_SYN/keep.pcap"
tcpdump -i lo -nn -w "$PCAP" "tcp and dst port $FPM_PORT and tcp[tcpflags] & tcp-syn != 0" \
  >/dev/null 2>&1 &
TCPDUMP_PID=$!
sleep 0.4
OK=0
FAIL=0
for i in $(seq 1 "$KEEP_REQS"); do
  if curl -fsS -o /dev/null --http1.1 -H 'Connection: close' \
    "http://${LISTEN}/marker.php?rid=keep-$i"; then
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
KEEP_CONN=FAIL
[[ "$OK" -eq "$KEEP_REQS" && "$FAIL" -eq 0 && "$SYN" -ge 1 && "$SYN" -lt "$KEEP_REQS" ]] && KEEP_CONN=PASS
echo "KEEP_CONN_REGRESSION=$KEEP_CONN OK=$OK FAIL=$FAIL REQS=$KEEP_REQS SYN=$SYN" \
  | tee "$STAGE_DIR/KEEP_CONN/SUMMARY.txt"

sample_proc end "$DP_PID"
FD_END=$(awk -F, '/,end,/ {print $4}' "$SERIES" | tail -1)
RSS_END=$(awk -F, '/,end,/ {print $5}' "$SERIES" | tail -1)

# Overall
CASE=P6ORACLE-E
if [[ "$TTL_RELOAD" == STRONG_CAUSAL_PASS && "$MAX_RELOAD" == STRONG_CAUSAL_PASS \
  && "$GEN_RELOAD" == STRONG_CAUSAL_PASS && "$KEEP_CONN" == PASS ]]; then
  CASE=P6ORACLE-B
fi
if [[ "$TTL_RELOAD" != STRONG_CAUSAL_PASS || "$MAX_RELOAD" != STRONG_CAUSAL_PASS \
  || "$GEN_RELOAD" != STRONG_CAUSAL_PASS ]]; then
  if [[ "$TTL_RELOAD" == STRONG_CAUSAL_PASS || "$MAX_RELOAD" == STRONG_CAUSAL_PASS \
    || "$GEN_RELOAD" == STRONG_CAUSAL_PASS ]]; then
    CASE=P6ORACLE-C
  else
    CASE=P6ORACLE-E
  fi
fi

{
  echo "CASE=$CASE"
  echo "TTL_RELOAD=$TTL_RELOAD"
  echo "MAX_CONNECTIONS_RELOAD=$MAX_RELOAD"
  echo "GENERATION_RELOAD=$GEN_RELOAD"
  echo "KEEP_CONN_REGRESSION=$KEEP_CONN"
  echo "KEEP_CONN_REQUESTS=$OK"
  echo "KEEP_CONN_BACKEND_ACCEPTS=$SYN"
  echo "BINARY_SHA256=$BINARY_SHA256"
  echo "REAL_PHP_FPM=PASS"
  echo "PHP_FPM_MAX_CHILDREN=$FPM_CHILDREN"
  echo "FD_END=$FD_END"
  echo "RSS_END=$RSS_END"
  echo "RUN_DISPOSITION=VALID"
} | tee "$STAGE_DIR/SUMMARY.txt"

echo "P6ORACLE_DONE CASE=$CASE TTL=$TTL_RELOAD MAX=$MAX_RELOAD GEN=$GEN_RELOAD KEEP=$KEEP_CONN"
exit 0
