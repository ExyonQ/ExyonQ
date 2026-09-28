#!/usr/bin/env bash
# Generic PHP front-controller + directory-index fixture (not WordPress).
# Proves ADR-043 repair is generic before WordPress requalification.
set -euo pipefail
ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
WORKDIR="${P6E_GENERIC_WORKDIR:-/tmp/exyonq-p6e-generic-fc-$$}"
# Unique ports per run (avoid collision with concurrent WP harness).
PORT_OFF=$(( $(date +%s) % 2000 ))
LISTEN="${P6E_GENERIC_LISTEN:-127.0.0.1:$((18100 + PORT_OFF))}"
FPM_PORT="${P6E_GENERIC_FPM_PORT:-$((19100 + PORT_OFF))}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase6e-wordpress-routing-repair}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID/GENERIC_PHP"
SKIP_CARGO_BUILD="${SKIP_CARGO_BUILD:-0}"
EXYONQ_DATAPLANE_BIN="${EXYONQ_DATAPLANE_BIN:-}"
CFD_PUBLISH_BIN="${CFD_PUBLISH_BIN:-}"
mkdir -p "$STAGE" "$WORKDIR/docroot/admin" "$WORKDIR/gen"
# Ensure generation dir exists before dataplane open (fail-closed if missing).
chmod 700 "$WORKDIR/gen" 2>/dev/null || true
# Seed empty generation so serve can start before first publish.
: >"$WORKDIR/gen/.keep"

DOCROOT="$WORKDIR/docroot"
cat >"$DOCROOT/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo "FC_ROOT\n";
echo "SCRIPT_FILENAME=".($_SERVER['SCRIPT_FILENAME']??'')."\n";
echo "SCRIPT_NAME=".($_SERVER['SCRIPT_NAME']??'')."\n";
echo "REQUEST_URI=".($_SERVER['REQUEST_URI']??'')."\n";
echo "QUERY_STRING=".($_SERVER['QUERY_STRING']??'')."\n";
PHP
cat >"$DOCROOT/admin/index.php" <<'PHP'
<?php
header('Content-Type: text/plain');
echo "DI_ADMIN\n";
echo "SCRIPT_FILENAME=".($_SERVER['SCRIPT_FILENAME']??'')."\n";
echo "SCRIPT_NAME=".($_SERVER['SCRIPT_NAME']??'')."\n";
echo "REQUEST_URI=".($_SERVER['REQUEST_URI']??'')."\n";
PHP
echo 'body{color:red}' >"$DOCROOT/style.css"
echo '<?php echo "LOGIN";' >"$DOCROOT/login.php"

PHP_FPM_BIN="$(command -v php-fpm8.3 || command -v php-fpm || true)"
[[ -n "$PHP_FPM_BIN" ]] || { echo "no php-fpm"; exit 2; }
FPM_USER=www-data
id "$FPM_USER" >/dev/null 2>&1 || FPM_USER="$(id -un)"
FPM_GROUP="$FPM_USER"
cat >"$WORKDIR/pool.conf" <<EOF
[www]
user = $FPM_USER
group = $FPM_GROUP
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = 2
clear_env = no
security.limit_extensions = .php
EOF
cat >"$WORKDIR/fpm.conf" <<EOF
[global]
pid = $WORKDIR/php-fpm.pid
error_log = $WORKDIR/php-fpm.log
daemonize = no
include = $WORKDIR/pool.conf
EOF

STATIC_PORT=$((FPM_PORT + 1))
cat >"$WORKDIR/routes.txt" <<EOF
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|1|60000|2000|30000|30000|60000
fcgi-dir-index|1|index.php
fcgi-front-controller|1|/index.php
|/style.css|127.0.0.1:${STATIC_PORT}|127.0.0.1
|/login.php|fcgi:1
|/admin|fcgi:1
|/|fcgi:1
EOF

source "${HOME}/.cargo/env" 2>/dev/null || true
cd "$ROOT"
if [[ "$SKIP_CARGO_BUILD" == "1" && -n "$EXYONQ_DATAPLANE_BIN" && -n "$CFD_PUBLISH_BIN" ]]; then
  BIN="$EXYONQ_DATAPLANE_BIN"
  PUB="$CFD_PUBLISH_BIN"
else
  cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never
  BIN="$ROOT/target/release/exyonq-dataplane"
  PUB="$ROOT/target/release/cfd-publish-routes"
fi
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL: missing binaries BIN=$BIN PUB=$PUB"; exit 3; }

"$PHP_FPM_BIN" -y "$WORKDIR/fpm.conf" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
python3 - <<PY &
import http.server, socketserver, os
os.chdir("$DOCROOT")
class H(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a): pass
with socketserver.TCPServer(("127.0.0.1", int("$STATIC_PORT")), H) as httpd:
    httpd.serve_forever()
PY
STATIC_PID=$!
sleep 0.4

# Publish BEFORE serve — dataplane requires an existing generation.bin (same as WP harness).
"$PUB" --gen-dir "$WORKDIR/gen" --routes "$WORKDIR/routes.txt" --generation-id 1

"$BIN" serve --listen "$LISTEN" --gen-dir "$WORKDIR/gen" --shards 1 --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
READY=0
for _ in $(seq 1 200); do
  if grep -q '^READY ' "$WORKDIR/gen/status" 2>/dev/null; then READY=1; break; fi
  if ! kill -0 "$DP_PID" 2>/dev/null; then
    echo "dataplane died"; cat "$WORKDIR/dp.err"; exit 3
  fi
  sleep 0.05
done
[[ "$READY" -eq 1 ]] || { echo "dataplane not READY"; cat "$WORKDIR/dp.err"; exit 3; }
sleep 0.2
# confirm accept
for _ in $(seq 1 50); do
  code=$(curl -sS -o /dev/null -w '%{http_code}' --connect-timeout 1 -H "Host: 127.0.0.1:${LISTEN##*:}" "http://${LISTEN}/login.php" || echo 000)
  [[ "$code" == "200" ]] && break
  sleep 0.1
done

HOST_H="Host: 127.0.0.1:${LISTEN##*:}"
req() {
  local path="$1" out="$2"
  curl -sS --max-redirs 0 -D "$STAGE/${out}.hdr" -o "$STAGE/${out}.body" \
    -H "$HOST_H" -H "Connection: close" "http://${LISTEN}${path}" || true
}

req "/pretty/path?x=1" fc_pretty
req "/admin/" di_admin
req "/style.css" static_css
req "/login.php" explicit_php
req "/missing-no-fc-would-404" fc_missing

kill "$DP_PID" "$FPM_PID" "$STATIC_PID" 2>/dev/null || true

{
  echo "GENERIC_FRONT_CONTROLLER=$(grep -q FC_ROOT "$STAGE/fc_pretty.body" && grep -q 'REQUEST_URI=/pretty/path' "$STAGE/fc_pretty.body" && grep -q 'SCRIPT_NAME=/index.php' "$STAGE/fc_pretty.body" && echo PASS || echo FAIL)"
  echo "GENERIC_DIRECTORY_INDEX=$(grep -q DI_ADMIN "$STAGE/di_admin.body" && grep -q 'SCRIPT_NAME=/admin/index.php' "$STAGE/di_admin.body" && grep -q 'REQUEST_URI=/admin/' "$STAGE/di_admin.body" && echo PASS || echo FAIL)"
  echo "GENERIC_STATIC=$(grep -q 'body{color:red}' "$STAGE/static_css.body" && echo PASS || echo FAIL)"
  echo "GENERIC_EXPLICIT_PHP=$(grep -q LOGIN "$STAGE/explicit_php.body" && echo PASS || echo FAIL)"
} | tee "$STAGE/RESULTS.env"

cat "$STAGE/RESULTS.env"
grep -q 'GENERIC_FRONT_CONTROLLER=PASS' "$STAGE/RESULTS.env"
grep -q 'GENERIC_DIRECTORY_INDEX=PASS' "$STAGE/RESULTS.env"
grep -q 'GENERIC_STATIC=PASS' "$STAGE/RESULTS.env"
grep -q 'GENERIC_EXPLICIT_PHP=PASS' "$STAGE/RESULTS.env"
echo "GENERIC_PHP_FIXTURE=PASS" | tee "$STAGE/SUMMARY.txt"
