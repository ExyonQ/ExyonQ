#!/usr/bin/env bash
# F6-WP-04: production-content migration gate.
#
# Copies a real WordPress tree and DB dump into a temporary directory, imports
# the DB into a disposable MariaDB container, serves the copy through ExyonQ's
# direct FastCGI route with PHP-FPM, and compares the rendered HTML with the
# production URL. Nginx is not started and the production tree/DB are read-only.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED platform=$(uname -s)"
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
WP_SOURCE="${WP_SOURCE_DIR:-/var/www/blog}"
WP_DUMP="${WP_DB_DUMP:-}"
PROD_URL="${PROD_BASE_URL:-https://blog.exyonq.org}"
WP_CLI="${WP_CLI:-wp}"
PHP_FPM_BIN="${PHP_FPM_BIN:-php-fpm8.5}"
DB_IMAGE="${DB_IMAGE:-mariadb:10.11}"

for t in curl openssl python3 docker "$WP_CLI" "$PHP_FPM_BIN"; do
  if ! command -v "$t" >/dev/null 2>&1; then
    echo "RESULT=BLOCKED missing=$t"
    exit 3
  fi
done
if [[ ! -x "$BIN" ]]; then
  echo "RESULT=BLOCKED missing=exyonq-binary path=$BIN"
  exit 3
fi
if [[ ! -d "$WP_SOURCE" ]]; then
  echo "RESULT=BLOCKED missing=wordpress-source path=$WP_SOURCE"
  exit 3
fi
if [[ -z "$WP_DUMP" || ! -f "$WP_DUMP" ]]; then
  echo "RESULT=BLOCKED missing=wordpress-db-dump (set WP_DB_DUMP)"
  exit 3
fi

pick_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

TMP="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-f6wp04.XXXXXX")"
DOC="$TMP/www"
SOCK="$TMP/php-fpm.sock"
FPM_CFG="$TMP/php-fpm.conf"
CFG="$TMP/exyonq.toml"
CERT="$TMP/cert.pem"
KEY="$TMP/key.pem"
EVIDENCE="${E2E_EV:-$ROOT/.exyonq-local/evidence/f6wp04-$(date -u +%Y%m%dT%H%M%SZ)}"
PORT="$(pick_port)"
DB_NAME="f6wp04"
DB_USER="f6wp04"
DB_PASS="$(openssl rand -hex 16)"
ADMIN_USER="f6wp04admin"
ADMIN_PASS="$(openssl rand -hex 16)"
DB_CID=""
FPM_PID=""
EX_PID=""
mkdir -p "$EVIDENCE" "$DOC" "$TMP/db-socket"
chmod 777 "$TMP/db-socket"
LOG="$EVIDENCE/setup.log"
: >"$LOG"

cleanup() {
  [[ -n "${EX_PID:-}" ]] && kill "$EX_PID" 2>/dev/null || true
  [[ -n "${FPM_PID:-}" ]] && kill "$FPM_PID" 2>/dev/null || true
  if [[ -n "${DB_CID:-}" ]]; then docker stop "$DB_CID" >/dev/null 2>&1 || true; fi
  wait 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*"
  echo "EVIDENCE=$EVIDENCE"
  [[ -f "$TMP/exyonq.log" ]] && cp "$TMP/exyonq.log" "$EVIDENCE/exyonq.log" 2>/dev/null || true
  [[ -f "$TMP/fpm.out" ]] && cp "$TMP/fpm.out" "$EVIDENCE/fpm.out" 2>/dev/null || true
  [[ -f "$TMP/php-fpm.log" ]] && cp "$TMP/php-fpm.log" "$EVIDENCE/php-fpm.log" 2>/dev/null || true
  tail -80 "$LOG" 2>/dev/null || true
  tail -80 "$EVIDENCE/exyonq.log" 2>/dev/null || true
  exit 1
}

echo "F6-WP-04 source=$WP_SOURCE binary=$BIN" | tee "$EVIDENCE/summary.txt"
"$BIN" --version 2>&1 | tee "$EVIDENCE/binary-version.txt"
"$PHP_FPM_BIN" -v 2>&1 | head -2 | tee "$EVIDENCE/php-fpm-version.txt"
sha256sum "$BIN" | tee "$EVIDENCE/binary.sha256"

echo "copying WordPress tree" | tee -a "$LOG"
if find "$WP_SOURCE" -type f ! -readable -print -quit | grep -q .; then
  # Production has a 0600 theme file owned by www-data. Use a read-only
  # container to copy it without changing production permissions.
  docker run --rm -v "$WP_SOURCE:/src:ro" cgr.dev/chainguard/wolfi-base:latest \
    tar -C /src -cf - . | tar --no-same-owner -C "$DOC" -xf - >>"$LOG" 2>&1 \
    || fail "could not copy unreadable production tree"
else
  cp -a "$WP_SOURCE/." "$DOC/"
fi
# This is a disposable copy; make it readable by the unprivileged FPM user.
chmod -R u+rwX,go+rX "$DOC"
chmod 755 "$TMP"

echo "starting disposable MariaDB" | tee -a "$LOG"
if ! docker image inspect "$DB_IMAGE" >/dev/null 2>&1; then
  docker pull "$DB_IMAGE" >>"$LOG" 2>&1 || fail "cannot pull $DB_IMAGE"
fi
DB_CID="$(docker run -d --rm --name "f6wp04-db-$$" \
  -e "MARIADB_ALLOW_EMPTY_ROOT_PASSWORD=yes" \
  -e "MARIADB_DATABASE=$DB_NAME" \
  -v "$TMP/db-socket:/run/mysqld" \
  "$DB_IMAGE")"
for _ in $(seq 1 90); do
  if docker exec "$DB_CID" mariadb-admin ping -uroot --silent >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
docker exec "$DB_CID" mariadb-admin ping -uroot --silent >/dev/null 2>&1 \
  || fail "MariaDB did not become ready"
docker exec "$DB_CID" chmod 777 /run/mysqld/mysqld.sock >/dev/null 2>&1 || fail "could not expose DB socket"
# Do not depend on image env initialization order: make the host-visible user
# and database explicit, then import as root into the disposable database.
if ! docker exec "$DB_CID" mariadb -uroot -e "
  CREATE DATABASE IF NOT EXISTS \`$DB_NAME\`;
  CREATE USER IF NOT EXISTS '$DB_USER'@'%' IDENTIFIED BY '$DB_PASS';
  ALTER USER '$DB_USER'@'%' IDENTIFIED BY '$DB_PASS';
  CREATE USER IF NOT EXISTS '$DB_USER'@'localhost' IDENTIFIED BY '$DB_PASS';
  ALTER USER '$DB_USER'@'localhost' IDENTIFIED BY '$DB_PASS';
  GRANT ALL PRIVILEGES ON \`$DB_NAME\`.* TO '$DB_USER'@'%';
  GRANT ALL PRIVILEGES ON \`$DB_NAME\`.* TO '$DB_USER'@'localhost';
  FLUSH PRIVILEGES;" >>"$LOG" 2>&1; then
  fail "could not provision disposable DB user"
fi
echo "importing real WordPress DB dump" | tee -a "$LOG"
if ! docker exec -i "$DB_CID" mariadb -uroot "$DB_NAME" <"$WP_DUMP" >>"$LOG" 2>&1; then
  fail "WordPress DB import failed"
fi

echo "configuring isolated WordPress copy" | tee -a "$LOG"
if ! "$WP_CLI" --path="$DOC" config set DB_NAME "$DB_NAME" --quiet >>"$LOG" 2>&1 \
  || ! "$WP_CLI" --path="$DOC" config set DB_USER "$DB_USER" --quiet >>"$LOG" 2>&1 \
  || ! "$WP_CLI" --path="$DOC" config set DB_PASSWORD "$DB_PASS" --quiet >>"$LOG" 2>&1 \
  || ! "$WP_CLI" --path="$DOC" config set DB_HOST "localhost:${TMP}/db-socket/mysqld.sock" --quiet >>"$LOG" 2>&1; then
  fail "wp config set failed"
fi
echo "db_host=$("$WP_CLI" --path="$DOC" config get DB_HOST --quiet 2>/dev/null)" | tee -a "$LOG"
if ! mysql --socket="$TMP/db-socket/mysqld.sock" -u"$DB_USER" -p"$DB_PASS" -N -e 'SELECT 1' >>"$LOG" 2>&1; then
  fail "direct mysql socket connection failed"
fi
if ! "$WP_CLI" --path="$DOC" db query 'SELECT 1' --skip-column-names >>"$LOG" 2>&1; then
  fail "WordPress cannot connect to disposable DB"
fi
SITE_URL_OUT="$("$WP_CLI" --path="$DOC" option get siteurl 2>>"$LOG")" \
  || fail "could not read siteurl from isolated WordPress"
SITE_URL="$(printf '%s\n' "$SITE_URL_OUT" | tail -1)"
[[ "$SITE_URL" == "$PROD_URL" ]] || fail "siteurl mismatch: $SITE_URL != $PROD_URL"
if ! "$WP_CLI" --path="$DOC" user create "$ADMIN_USER" "$ADMIN_USER@example.test" \
  --role=administrator --user_pass="$ADMIN_PASS" --quiet >>"$LOG" 2>&1; then
  fail "could not create disposable admin"
fi
POST_COUNT_OUT="$("$WP_CLI" --path="$DOC" post list --post_type=post --format=count 2>>"$LOG")" \
  || fail "could not count posts from isolated WordPress"
POST_COUNT="$(printf '%s\n' "$POST_COUNT_OUT" | tail -1)"
[[ "$POST_COUNT" -ge 2 ]] || fail "not enough real posts: $POST_COUNT"
echo "siteurl=$SITE_URL posts=$POST_COUNT admin=$ADMIN_USER" | tee -a "$LOG"

openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 2 \
  -subj "/CN=blog.exyonq.org" \
  -addext "subjectAltName=DNS:blog.exyonq.org" \
  -keyout "$KEY" -out "$CERT" >/dev/null 2>&1 || fail "ephemeral TLS cert failed"
chmod 600 "$KEY"
chmod 644 "$CERT"

FPM_USER="$(id -un)"
FPM_GROUP="$(id -gn)"
if [[ "$(id -u)" == "0" ]]; then
  FPM_USER="${FPM_USER_OVERRIDE:-www-data}"
  FPM_GROUP="${FPM_GROUP_OVERRIDE:-www-data}"
  chown -R "$FPM_USER:$FPM_GROUP" "$DOC" "$TMP"
fi
cat >"$FPM_CFG" <<EOF
[global]
error_log = $TMP/php-fpm.log
daemonize = no
[www]
user = $FPM_USER
group = $FPM_GROUP
listen = $SOCK
listen.owner = $FPM_USER
listen.group = $FPM_GROUP
listen.mode = 0666
pm = static
pm.max_children = 20
clear_env = no
catch_workers_output = yes
EOF
"$PHP_FPM_BIN" -t -y "$FPM_CFG" >/dev/null 2>>"$LOG" || fail "php-fpm config invalid"
"$PHP_FPM_BIN" --nodaemonize --fpm-config "$FPM_CFG" >"$TMP/fpm.out" 2>&1 &
FPM_PID=$!
for _ in $(seq 1 100); do [[ -S "$SOCK" ]] && break; sleep 0.1; done
[[ -S "$SOCK" ]] || fail "php-fpm socket missing"

cat >"$CFG" <<EOF
config_version = 2

[logging]
level = "info"
format = "text"

[logging.console]
enabled = true
stream = "stdout"

[logging.access]
enabled = true

[modules.compression]
enabled = true
min_bytes = 256

[[server]]
listen = "127.0.0.1:${PORT}"
server_name = ["blog.exyonq.org"]
routes = ["wp-includes", "wp-content", "wp-config-deny", "wordpress"]

[server.tls]
cert = "${CERT}"
key = "${KEY}"

[[route]]
name = "wp-includes"
match = { path = "/wp-includes" }
root = "${DOC}/wp-includes"
cache_headers = { by_mime = { "*" = { max_age = 2592000, immutable = true } } }

[[route]]
name = "wp-content"
match = { path = "/wp-content" }
root = "${DOC}/wp-content"
cache_headers = { by_mime = { "*" = { max_age = 2592000, immutable = true } } }

[[route]]
name = "wp-config-deny"
match = { path = "/wp-config.php" }
access = { deny = ["all"] }

[[route]]
name = "wordpress"
match = { path = "/" }
fastcgi = "php"
# Production has no .htaccess (nginx uses try_files). This is the equivalent:
# existing files/directory indexes win; every other URI runs /index.php.
front_controller = "index.php"

[[fcgi_pool]]
name = "php"
address = "${SOCK}"
transport = "unix"
document_root = "${DOC}"
max_concurrency = 24
max_connections = 24
idle_timeout_ms = 30000
total_timeout_ms = 120000
checkout_timeout_ms = 5000

[full_page_cache]
enabled = false
EOF

EXYONQ_CONTROL_SOCKET="$TMP/control.sock" \
  "$BIN" serve --config "$CFG" >"$TMP/exyonq.log" 2>&1 &
EX_PID=$!
CURL=(curl -k -sS --max-time 30 --resolve "blog.exyonq.org:${PORT}:127.0.0.1" -H 'Host: blog.exyonq.org')
READY=0
for _ in $(seq 1 120); do
  CODE="$("${CURL[@]}" -o /dev/null -w '%{http_code}' "https://blog.exyonq.org:${PORT}/" 2>/dev/null || true)"
  if [[ "$CODE" == "200" || "$CODE" == "301" || "$CODE" == "302" ]]; then READY=1; break; fi
  if ! kill -0 "$EX_PID" 2>/dev/null; then
    fail "ExyonQ exited during startup (last HTTP=$CODE)"
  fi
  sleep 0.25
done
[[ "$READY" -eq 1 ]] || fail "ExyonQ readiness timeout (last HTTP=$CODE)"

status_for() {
  "${CURL[@]}" -o "$TMP/body" -w '%{http_code}' "$1" 2>/dev/null || true
}
assert_status() {
  local path="$1" expected="$2" code
  code="$(status_for "https://blog.exyonq.org:${PORT}${path}")"
  [[ "$code" == "$expected" ]] || fail "${path} expected ${expected}, got ${code}"
  echo "PASS ${path} -> ${code}" | tee -a "$LOG"
}
assert_contains() {
  local path="$1" needle="$2"
  "${CURL[@]}" -o "$TMP/body" "https://blog.exyonq.org:${PORT}${path}" >/dev/null
  grep -Fq "$needle" "$TMP/body" || fail "${path} missing '${needle}'"
  echo "PASS ${path} contains ${needle}" | tee -a "$LOG"
}
assert_cache() {
  local path="$1"
  "${CURL[@]}" -D "$TMP/headers" -o "$TMP/body" "https://blog.exyonq.org:${PORT}${path}" >/dev/null
  tr -d '\r' <"$TMP/headers" | grep -Fqi 'cache-control: public, max-age=2592000, immutable' \
    || fail "${path} missing 30-day immutable Cache-Control"
  echo "PASS ${path} cache-control" | tee -a "$LOG"
}

assert_status "/" "200"
assert_contains "/" "<title>ExyonQ</title>"
assert_status "/categoria/releases/" "200"
assert_contains "/categoria/releases/" "Releases"
assert_status "/exyonq-0-9-0/" "200"
assert_contains "/exyonq-0-9-0/" "ExyonQ 0.9.0"
assert_status "/f6wp04-no-existe/" "404"
assert_status "/feed/" "200"
assert_contains "/feed/" "<rss"
assert_status "/wp-json/" "200"
assert_contains "/wp-json/" '"name":"ExyonQ"'
assert_status "/wp-config.php" "403"
assert_status "/wp-config.php?probe=1" "403"
assert_status "/wp-config.php/" "403"
assert_cache "/wp-includes/js/wp-emoji-release.min.js?ver=7.1.4"
assert_cache "/wp-content/themes/exyonq/style.css?ver=1.0.0"
assert_cache "/wp-content/uploads/2026/11/exyonq-site-icon.png"

# A logged-out admin URL must redirect to the HTTPS login URL, not to HTTP.
ADMIN_CODE="$("${CURL[@]}" -D "$TMP/admin.headers" -o /dev/null -w '%{http_code}' \
  "https://blog.exyonq.org:${PORT}/wp-admin/")"
[[ "$ADMIN_CODE" == "302" ]] || fail "/wp-admin/ expected 302, got $ADMIN_CODE"
ADMIN_LOC="$(tr -d '\r' <"$TMP/admin.headers" | sed -n 's/^[Ll]ocation: //p' | head -1)"
[[ "$ADMIN_LOC" == "https://blog.exyonq.org/wp-login.php?redirect_to=https%3A%2F%2Fblog.exyonq.org%2Fwp-admin%2F&reauth=1" ]] \
  || fail "/wp-admin/ redirect is not the canonical HTTPS URL: $ADMIN_LOC"
echo "PASS /wp-admin/ -> 302 $ADMIN_LOC" | tee -a "$LOG"

LOGIN_CODE="$("${CURL[@]}" -o "$TMP/login.html" -w '%{http_code}' \
  "https://blog.exyonq.org:${PORT}/wp-login.php")"
case "$LOGIN_CODE" in
  200) grep -qi 'name="log"' "$TMP/login.html" || fail "wp-login form missing";;
  302) ADMIN_LOC="$(tr -d '\r' <"$TMP/admin.headers" | sed -n 's/^[Ll]ocation: //p' | head -1)";;
  *) fail "/wp-login.php unexpected status $LOGIN_CODE";;
esac
echo "PASS /wp-login.php -> $LOGIN_CODE" | tee -a "$LOG"

# Log in with a disposable admin created in the copied DB.
"${CURL[@]}" -c "$TMP/cookies" -b "$TMP/cookies" -o "$TMP/login.html" \
  "https://blog.exyonq.org:${PORT}/wp-login.php" >/dev/null
DASH_CODE="$(curl -k -sS --max-time 30 --resolve "blog.exyonq.org:${PORT}:127.0.0.1" \
  -L -c "$TMP/cookies" -b "$TMP/cookies" \
  --data-urlencode "log=$ADMIN_USER" \
  --data-urlencode "pwd=$ADMIN_PASS" \
  --data-urlencode "wp-submit=Log In" \
  --data-urlencode "redirect_to=https://blog.exyonq.org/wp-admin/" \
  --data-urlencode "testcookie=1" \
  -o "$TMP/dashboard.html" -w '%{http_code}' \
  "https://blog.exyonq.org:${PORT}/wp-login.php")"
[[ "$DASH_CODE" == "200" ]] || fail "login ended with $DASH_CODE"
grep -Eqi 'Dashboard|wp-admin-bar|wp-admin' "$TMP/dashboard.html" || fail "login did not reach dashboard"
echo "PASS disposable admin login -> $DASH_CODE" | tee -a "$LOG"

# Brotli must be negotiated and the decoded HTML must match production.
"${CURL[@]}" -H 'Accept-Encoding: br' -D "$TMP/cand-br.headers" -o "$TMP/cand.br" \
  "https://blog.exyonq.org:${PORT}/" >/dev/null
grep -Fqi 'content-encoding: br' "$TMP/cand-br.headers" || fail "candidate did not return br"
"${CURL[@]}" --compressed -o "$TMP/candidate.html" "https://blog.exyonq.org:${PORT}/" >/dev/null
curl -sS -L --compressed --max-time 30 "$PROD_URL" -o "$TMP/production.html" \
  || fail "production fetch failed"
if cmp -s "$TMP/candidate.html" "$TMP/production.html"; then
  echo "PASS HTML byte-identical to production" | tee -a "$LOG"
  echo "html_mode=byte-identical" >>"$EVIDENCE/summary.txt"
else
  diff -u "$TMP/production.html" "$TMP/candidate.html" >"$EVIDENCE/html-raw.diff" || true
  # ONE optional core hint is generated only by the isolated copy. Strip only
  # that exact line, then require every remaining byte to match.
  sed "/<link rel='dns-prefetch' href='\/\/blog\.exyonq\.org' \/>/d" \
    "$TMP/candidate.html" >"$TMP/candidate.norm.html"
  sed "/<link rel='dns-prefetch' href='\/\/blog\.exyonq\.org' \/>/d" \
    "$TMP/production.html" >"$TMP/production.norm.html"
  cmp -s "$TMP/candidate.norm.html" "$TMP/production.norm.html" \
    || fail "candidate HTML differs beyond the optional dns-prefetch hint"
  echo "PASS HTML matches production after removing optional dns-prefetch" | tee -a "$LOG"
  echo "html_mode=normalized-dns-prefetch" >>"$EVIDENCE/summary.txt"
fi
# HEAD has no body and often no Content-Length. Measure a real GET and keep
# the production encoding separate: the candidate must negotiate br, while
# production may be served by nginx with a different encoding policy.
PROD_BR_BYTES="$(curl -sS -L -H 'Accept-Encoding: br' --max-time 30 \
  -D "$TMP/prod-br.headers" -o "$TMP/prod.br" -w '%{size_download}' "$PROD_URL" \
  || fail "production brotli fetch failed")"
PROD_ENC="$(tr -d '\r' <"$TMP/prod-br.headers" | awk 'tolower($1)=="content-encoding:"{print $2; exit}')"
[[ -n "$PROD_ENC" ]] || PROD_ENC=none
printf 'candidate_br_bytes=%s production_br_bytes=%s production_content_encoding=%s\n' \
  "$(wc -c <"$TMP/cand.br")" "$PROD_BR_BYTES" "$PROD_ENC" | tee -a "$LOG"
echo "production_br_bytes=$PROD_BR_BYTES" >>"$EVIDENCE/summary.txt"
echo "production_content_encoding=$PROD_ENC" >>"$EVIDENCE/summary.txt"
echo "candidate_br_sha256=$(sha256sum "$TMP/cand.br" | awk '{print $1}')" >>"$EVIDENCE/summary.txt"
echo "production_br_sha256=$(sha256sum "$TMP/prod.br" | awk '{print $1}')" >>"$EVIDENCE/summary.txt"
if cmp -s "$TMP/cand.br" "$TMP/prod.br"; then
  echo "compressed_html_byte_identical=yes" >>"$EVIDENCE/summary.txt"
else
  echo "compressed_html_byte_identical=no (decoded HTML was compared above)" >>"$EVIDENCE/summary.txt"
fi

# 20 concurrent dynamic requests must all succeed (FPM ceiling is 20).
rm -f "$TMP/codes"
PIDS=()
for i in $(seq 1 20); do
  ( "${CURL[@]}" -o /dev/null -w '%{http_code}\n' \
      "https://blog.exyonq.org:${PORT}/exyonq-0-9-0/?f6wp04=$i" >>"$TMP/codes" ) &
  PIDS+=("$!")
done
for pid in "${PIDS[@]}"; do wait "$pid"; done
OK_COUNT="$(grep -c '^200$' "$TMP/codes" || true)"
[[ "$OK_COUNT" -eq 20 ]] || { cat "$TMP/codes"; fail "concurrency: ${OK_COUNT}/20 returned 200"; }
echo "PASS concurrency 20/20 200" | tee -a "$LOG"

cp "$TMP/candidate.html" "$TMP/production.html" "$TMP/cand.br" "$TMP/admin.headers" \
  "$TMP/cand-br.headers" "$EVIDENCE/" 2>/dev/null || true
{
  echo "siteurl=$SITE_URL"
  echo "posts=$POST_COUNT"
  echo "port=$PORT"
  echo "admin_redirect=$ADMIN_LOC"
  echo "wp_login_status=$LOGIN_CODE"
  echo "login_status=$DASH_CODE"
  echo "concurrency_200=$OK_COUNT/20"
  echo "candidate_html_sha256=$(sha256sum "$TMP/candidate.html" | awk '{print $1}')"
  echo "production_html_sha256=$(sha256sum "$TMP/production.html" | awk '{print $1}')"
  echo "candidate_br_bytes=$(wc -c <"$TMP/cand.br")"
} >>"$EVIDENCE/summary.txt"
echo "RESULT=PASS f6-wp04 production content direct FastCGI"
echo "EVIDENCE=$EVIDENCE"
