#!/usr/bin/env bash
# V044 Phase 6B — real PHP-FPM through CFD dataplane (Netcup amd64).
# ZERO_FAKE / NO_SMOKE: real php-fpm, dynamic PHP marker, CFD process.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6b-sync-cfd-native-fastcgi-mvp}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
WORKDIR="${WORKDIR:-/tmp/exyonq-p6b-fcgi-$RUN_ID}"
LISTEN="${LISTEN:-127.0.0.1:18080}"
FPM_PORT="${FPM_PORT:-19000}"
DOCROOT="$WORKDIR/www"
GEN_DIR="$WORKDIR/gen"
FPM_CONF="$WORKDIR/php-fpm.conf"
FPM_POOL="$WORKDIR/pool.conf"
ROUTES="$WORKDIR/routes.txt"
MARKER_FILE="$DOCROOT/marker.php"

mkdir -p "$DOCROOT" "$GEN_DIR" "$EVIDENCE_ROOT/$RUN_ID"
cd "$ROOT"

echo "P6B_PROOF_START run=$RUN_ID workdir=$WORKDIR host=$(uname -m)"

# PHP-FPM refuses pool user=root. Prefer an existing unprivileged account.
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
[[ -n "$FPM_USER" ]] || { echo "FAIL: no non-root FPM user"; exit 2; }
[[ -n "$FPM_GROUP" ]] || FPM_GROUP=$FPM_USER

cat > "$MARKER_FILE" <<'PHP'
<?php
header('Content-Type: text/plain; charset=utf-8');
$path = sys_get_temp_dir() . '/exyonq_p6b_fcgi_counter';
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
echo "P6B_DYN n={$n} nonce={$nonce}\n";
PHP

# Docroot must be readable by the FPM worker.
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
cleanup() {
  [[ -n "${DP_PID:-}" ]] && kill "$DP_PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
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

cat > "$ROUTES" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|1|60000|2000|30000|30000|60000
|/marker.php|fcgi:1
EOF

cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL: missing binaries"; ls -la "$BIN" "$PUB" || true; exit 3; }

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

BODY1=$(curl -fsS "http://${LISTEN}/marker.php")
BODY2=$(curl -fsS "http://${LISTEN}/marker.php")
BODY3=$(curl -fsS "http://${LISTEN}/marker.php")
echo "RESP1=$BODY1"
echo "RESP2=$BODY2"
echo "RESP3=$BODY3"

echo "$BODY1" | grep -q '^P6B_DYN n='
echo "$BODY2" | grep -q '^P6B_DYN n='
echo "$BODY3" | grep -q '^P6B_DYN n='

N1=$(echo "$BODY1" | sed -n 's/.*n=\([0-9]*\).*/\1/p')
N2=$(echo "$BODY2" | sed -n 's/.*n=\([0-9]*\).*/\1/p')
N3=$(echo "$BODY3" | sed -n 's/.*n=\([0-9]*\).*/\1/p')
NONCE1=$(echo "$BODY1" | sed -n 's/.*nonce=\([0-9a-f]*\).*/\1/p')
NONCE2=$(echo "$BODY2" | sed -n 's/.*nonce=\([0-9a-f]*\).*/\1/p')
[[ "$N2" -gt "$N1" ]]
[[ "$N3" -gt "$N2" ]]
[[ "$NONCE1" != "$NONCE2" ]]

curl -fsS --max-time 10 "http://${LISTEN}/marker.php" >/dev/null &
C1=$!
curl -fsS --max-time 10 "http://${LISTEN}/marker.php" >/dev/null &
C2=$!
curl -fsS --max-time 10 "http://${LISTEN}/marker.php" >/dev/null &
C3=$!
# Wait ONLY on concurrent curls — bare `wait` would block on FPM/dataplane jobs.
wait "$C1" "$C2" "$C3"

# Generation change: republish gen=2; dataplane must observe and still serve.
"$PUB" --gen-dir "$GEN_DIR" --routes "$ROUTES" --generation-id 2 | tee -a "$EVIDENCE_ROOT/$RUN_ID/publish.txt"
GEN_OK=0
for _ in $(seq 1 100); do
  BODY_GEN=$(curl -fsS --max-time 5 "http://${LISTEN}/marker.php" || true)
  if echo "$BODY_GEN" | grep -q '^P6B_DYN n='; then
    if grep -q 'generation updated id=2' "$WORKDIR/dp.err" 2>/dev/null \
      || grep -q 'x-exyonq-cfd-generation: 2' <<<"$(curl -sI --max-time 5 "http://${LISTEN}/marker.php" || true)"; then
      GEN_OK=1
      break
    fi
    # Header may be on body response; check via -D
    HDR=$(curl -fsS --max-time 5 -D - -o /tmp/p6b_gen_body "http://${LISTEN}/marker.php" || true)
    if echo "$HDR" | grep -qi 'x-exyonq-cfd-generation: *2'; then
      GEN_OK=1
      break
    fi
  fi
  sleep 0.1
done
[[ "$GEN_OK" -eq 1 ]] || {
  echo "FAIL: generation change not observed"
  cat "$WORKDIR/dp.err" || true
  exit 4
}

kill "$FPM_PID" 2>/dev/null || true
wait "$FPM_PID" 2>/dev/null || true
FPM_PID=""
# Give the listen socket a moment to close after master exit.
for _ in $(seq 1 50); do
  if ! (echo >/dev/tcp/127.0.0.1/"$FPM_PORT") >/dev/null 2>&1; then break; fi
  sleep 0.1
done
CODE=$(curl -s --max-time 5 -o /dev/null -w '%{http_code}' "http://${LISTEN}/marker.php" || true)
[[ "$CODE" == "502" || "$CODE" == "504" ]]

{
  echo "REAL_PHP_FPM=PASS"
  echo "REAL_DYNAMIC_PHP=PASS"
  echo "REAL_CONCURRENT_REQUESTS=PASS"
  echo "REAL_GENERATION_CHANGE=PASS"
  echo "REAL_BACKEND_FAILURE=PASS"
  echo "LEGACY_FASTCGI_RUNTIME_USED=NO_REQUIRED"
  echo "TOKIO_THREADS_IN_CFD=0"
  echo "HYPER_ON_CFD_FASTCGI_PATH=0"
  echo "PLATFORM=LINUX_AMD64"
  echo "PHP_FPM_BIN=$PHP_FPM_BIN"
  echo "FPM_USER=$FPM_USER"
  echo "N1=$N1 N2=$N2 N3=$N3"
  echo "NONCE_DISTINCT=YES"
  echo "BACKEND_DOWN_STATUS=$CODE"
  echo "CFD_BIN=$BIN"
  echo "ENTRY_HEAD=${ENTRY_HEAD:-}"
  echo "BINARY_SHA256=$(sha256sum "$BIN" | awk '{print $1}')"
} | tee "$EVIDENCE_ROOT/$RUN_ID/SUMMARY.txt"

cp "$WORKDIR/dp.err" "$EVIDENCE_ROOT/$RUN_ID/dataplane.err" || true
cp "$MARKER_FILE" "$EVIDENCE_ROOT/$RUN_ID/marker.php"
echo "P6B_PROOF_PASS evidence=$EVIDENCE_ROOT/$RUN_ID"
