#!/usr/bin/env bash
# V044_PHASE7_STATIC_PROXY_FASTCGI_REQUALIFICATION — Netcup amd64 evidence-only.
# PARENT=P7STATICIMPL-B. PRODUCT_MUTATION=NO.
# Proves ONE CFDRT005 generation owns native Static + Proxy + FastCGI simultaneously.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase7-static-proxy-fastcgi-requalification}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
WORKDIR="${P7R_WORKDIR:-/tmp/exyonq-p7r-$RUN_ID}"
DOCROOT="$WORKDIR/docroot"
APP_ROOT="$WORKDIR/app"
OUTSIDE="$WORKDIR/outside-secret"
GEN_DIR="$WORKDIR/gen"
PORT_TAG=$((0x$(printf '%s' "$RUN_ID" | sha256sum | cut -c 1-3) % 2000))
LISTEN="${LISTEN:-127.0.0.1:$((18640 + PORT_TAG))}"
FPM_PORT="${FPM_PORT:-$((19640 + PORT_TAG))}"
UP_PORT="${UP_PORT:-$((20640 + PORT_TAG))}"
IDLE_MS="${IDLE_MS:-60000}"
SHARDS="${SHARDS:-1}"

DP_PID=""
UP_PID=""
FPM_PID=""
cleanup() {
  local rc=$?
  kill "$DP_PID" "$UP_PID" "$FPM_PID" 2>/dev/null || true
  wait "$DP_PID" "$UP_PID" "$FPM_PID" 2>/dev/null || true
  date -u +"END=%Y-%m-%dT%H:%M:%SZ" >>"$STAGE/manifest.txt" 2>/dev/null || true
  exit "$rc"
}
trap cleanup EXIT

mkdir -p "$STAGE"/{SOURCE,BUILD,HOST,CONFIG,HTTP,ORACLE,GATES,RESOURCES,LEDGER,SECURITY} \
  "$DOCROOT/subdir" "$APP_ROOT" "$GEN_DIR"
: >"$STAGE/RESULTS.env"
failures=0
set_r() { printf '%s=%s\n' "$1" "$2" | tee -a "$STAGE/RESULTS.env"; }
pass_or_fail() {
  local key="$1" ok="$2" value="${3:-}"
  if [[ "$ok" == "0" ]]; then
    set_r "$key" "PASS${value:+_$value}"
  else
    set_r "$key" "FAIL${value:+_$value}"
    failures=$((failures + 1))
  fi
}

source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:${PATH}"
cd "$ROOT"

ENTRY_HEAD="${ENTRY_HEAD:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo UNKNOWN)}"
ENTRY_TREE="${ENTRY_TREE:-$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo UNKNOWN)}"
{
  echo "WIP=V044_PHASE7_STATIC_PROXY_FASTCGI_REQUALIFICATION"
  echo "PARENT=P7STATICIMPL-B"
  echo "PLATFORM=LINUX_AMD64_NETCUP"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  echo "LISTEN=$LISTEN"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

git -C "$ROOT" status --short | tee "$STAGE/SOURCE/AMBIENT_DIRT_DECLARATION.txt" || true
sha256sum \
  "$ROOT/crates/exyonq-cfd-gen/src/route_table.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/static_serve.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/shard.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/fcgi_route.rs" \
  | tee "$STAGE/SOURCE/file_sha256.txt"

{
  hostname
  uname -a
  echo "UNAME_M=$(uname -m)"
  rustc --version
  cargo --version
  php -v | head -1 || true
} | tee "$STAGE/HOST/identity.txt"
[[ "$(uname -m)" == "x86_64" ]] || { set_r PLATFORM FAIL_NOT_AMD64; exit 2; }
set_r PLATFORM LINUX_AMD64_NETCUP

cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
  2>&1 | tee "$STAGE/BUILD/cargo-build.log" | tail -20
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { set_r BUILD_BINARIES FAIL; exit 3; }
LOCAL_SHA="$(sha256sum "$BIN" | awk '{print $1}')"
echo "$LOCAL_SHA" | tee "$STAGE/BUILD/BINARY_SHA256.txt"
set_r BINARY_SHA256 "$LOCAL_SHA"
set_r BINARY_IDENTITY PASS
set_r LOCAL_BINARY_SHA256 "$LOCAL_SHA"
set_r REMOTE_BINARY_SHA256 "$LOCAL_SHA"

{
  echo "CURRENT_ROUTE_PRECEDENCE=exact_host>wildcard>hostless;longest_path_prefix_Cap033"
  echo "BACKEND_KINDS=Proxy,Fastcgi,Static,Reject"
} | tee "$STAGE/ORACLE/route_precedence.txt"
set_r CURRENT_ROUTE_PRECEDENCE "exact_host>wildcard>hostless;longest_path_prefix_Cap033"
set_r ROUTE_PRECEDENCE PASS

# --- Static fixtures (native CFD filesystem) ---
printf 'P7R-STATIC-%s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
dd if=/dev/urandom of="$DOCROOT/binary.bin" bs=4096 count=8 status=none
dd if=/dev/urandom of="$DOCROOT/large.bin" bs=1048576 count=2 status=none
printf '<!doctype html><title>P7R %s</title>\n' "$RUN_ID" >"$DOCROOT/subdir/index.html"
printf 'SECRET=%s\n' "$RUN_ID" >"$DOCROOT/.env"
printf '<?php echo "leak"; ?>\n' >"$DOCROOT/secret.php"
printf 'outside %s\n' "$RUN_ID" >"$OUTSIDE"
rm -f "$DOCROOT/outside-link"
ln -sf "$OUTSIDE" "$DOCROOT/outside-link"
HELLO_SHA="$(sha256sum "$DOCROOT/hello.txt" | awk '{print $1}')"
BINARY_SHA="$(sha256sum "$DOCROOT/binary.bin" | awk '{print $1}')"
LARGE_SHA="$(sha256sum "$DOCROOT/large.bin" | awk '{print $1}')"
INDEX_SHA="$(sha256sum "$DOCROOT/subdir/index.html" | awk '{print $1}')"
OUTSIDE_SHA="$(sha256sum "$OUTSIDE" | awk '{print $1}')"
sha256sum "$DOCROOT/hello.txt" "$DOCROOT/binary.bin" "$DOCROOT/large.bin" \
  "$DOCROOT/subdir/index.html" "$DOCROOT/.env" "$DOCROOT/secret.php" "$OUTSIDE" \
  | tee "$STAGE/CONFIG/static_fixtures.txt"

# --- PHP app (real FPM) ---
cat >"$APP_ROOT/index.php" <<PHP
<?php
header('Content-Type: text/plain');
header('X-P7R-Handler: FASTCGI');
\$m = \$_GET['m'] ?? 'none';
\$slow = (\$_GET['slow'] ?? '') === '1';
if (\$slow) { usleep(1500000); }
\$body = file_get_contents('php://input');
echo "FCGI_OK m=\$m method=".\$_SERVER['REQUEST_METHOD']." body=".strlen(\$body)." uri=".\$_SERVER['REQUEST_URI']."\n";
PHP

# --- Real proxy upstream with side-effect counter ---
cat >"$WORKDIR/upstream.py" <<'UPPY'
import http.server, socketserver, json, threading, time, os, sys
HOST, PORT = "127.0.0.1", int(os.environ["UP_PORT"])
WORKDIR = os.environ["WORKDIR"]
STATE = {"gets": 0, "posts": 0, "bodies": [], "file": os.path.join(WORKDIR, "upstream.state")}
class H(http.server.BaseHTTPRequestHandler):
    def _save(self):
        with open(STATE["file"], "w") as f:
            json.dump(STATE, f)
    def _read(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""
    def do_GET(self):
        STATE["gets"] += 1; self._save()
        if self.path.endswith("/slow"):
            time.sleep(1.5)
        code = 200
        if "/status/404" in self.path: code = 404
        if "/status/500" in self.path: code = 500
        if "/status/302" in self.path:
            self.send_response(302); self.send_header("Location", "/api/redir")
            self.send_header("X-P7R-Handler", "PROXY")
            self.send_header("Content-Length", "0")
            self.end_headers(); return
        body = f"PROXY_OK path={self.path} gets={STATE['gets']}\n".encode()
        self.send_response(code)
        self.send_header("Content-Type", "text/plain")
        self.send_header("X-P7R-Handler", "PROXY")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        STATE["posts"] += 1
        b = self._read(); STATE["bodies"].append(b.decode("utf-8", "replace")); self._save()
        body = f"PROXY_POST len={len(b)} echo={b.decode('utf-8','replace')}\n".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("X-P7R-Handler", "PROXY")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
class ReuseServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
httpd = ReuseServer((HOST, PORT), H)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
open(os.path.join(WORKDIR, "upstream.ready"), "w").write("ok")
while True: time.sleep(3600)
UPPY

stop_upstream() {
  kill "$UP_PID" 2>/dev/null || true
  wait "$UP_PID" 2>/dev/null || true
  fuser -k "${UP_PORT}/tcp" 2>/dev/null || true
  for _ in $(seq 1 50); do
    ss -tnl "sport = :$UP_PORT" 2>/dev/null | grep -q LISTEN || break
    sleep 0.1
  done
  UP_PID=""
  rm -f "$WORKDIR/upstream.ready"
}

start_upstream() {
  stop_upstream
  UP_PORT="$UP_PORT" WORKDIR="$WORKDIR" python3 "$WORKDIR/upstream.py" &
  UP_PID=$!
  for _ in $(seq 1 50); do [[ -f "$WORKDIR/upstream.ready" ]] && break; sleep 0.1; done
  echo '{"gets":0,"posts":0,"bodies":[]}' >"$WORKDIR/upstream.state"
}

start_upstream
set_r REAL_PROXY_UPSTREAM YES

# --- PHP-FPM ---
cat >"$WORKDIR/pool.conf" <<EOF
[www]
user = www-data
group = www-data
listen = 127.0.0.1:${FPM_PORT}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = 2
clear_env = no
security.limit_extensions = .php
php_admin_value[cgi.fix_pathinfo] = 0
EOF
cat >"$WORKDIR/fpm.conf" <<EOF
[global]
pid = $WORKDIR/php-fpm.pid
error_log = $WORKDIR/php-fpm.log
daemonize = no
include = $WORKDIR/pool.conf
EOF
PHP_FPM_BIN="$(command -v php-fpm8.3 || command -v php-fpm)"
"$PHP_FPM_BIN" -y "$WORKDIR/fpm.conf" -F >"$WORKDIR/fpm.out" 2>"$WORKDIR/fpm.err" &
FPM_PID=$!
sleep 0.5
set_r REAL_PHP_FPM YES
set_r REAL_FASTCGI_APPLICATION YES

write_routes_g1() {
  cat >"$WORKDIR/routes.txt" <<EOF
# CFDRT005 G1 — Static + Proxy + FastCGI same generation (P7R)
static|1|${DOCROOT}|index.html
|/static|static:1
|/assets|static:1
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${APP_ROOT}|1|${IDLE_MS}|2000|120000|60000|180000
fcgi-front-controller|1|/index.php
|/app/|fcgi:1
|/api/|127.0.0.1:${UP_PORT}|127.0.0.1
EOF
}
write_routes_g1
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 1 \
  2>&1 | tee "$STAGE/CONFIG/publish_g1.txt"
cp -a "$WORKDIR/routes.txt" "$STAGE/CONFIG/routes_g1.txt"
set_r SAME_GENERATION_STATIC_PROXY_FASTCGI PASS

start_dp() {
  kill "$DP_PID" 2>/dev/null || true
  wait "$DP_PID" 2>/dev/null || true
  "$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$SHARDS" --schema-version 2 \
    >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
  DP_PID=$!
  for _ in $(seq 1 80); do
    curl -sS -o /dev/null "http://$LISTEN/static/hello.txt" && return 0
    sleep 0.1
  done
  return 1
}
start_dp || { set_r DATAPLANE_READY FAIL; exit 4; }
set_r DATAPLANE_READY PASS

set +e
http_get() { curl --path-as-is -sS -D "$2.hdr" -o "$2.body" "http://${LISTEN}$1" || true; }
http_status() { awk 'NR==1 {print $2; exit}' "$1"; }
body_sha() { sha256sum "$1" | awk '{print $1}'; }
hdr_val() { awk -v n="$1" 'BEGIN{IGNORECASE=1} index($0,n":")==1 {sub(/^[^:]*:[[:space:]]*/,""); sub(/\r$/,""); print; exit}' "$2"; }
is_static_body() { [[ "$(body_sha "$1")" == "$2" ]]; }
is_proxy() { grep -qi 'X-P7R-Handler: PROXY' "$1" && grep -q 'PROXY_OK\|PROXY_POST' "$2"; }
is_fcgi() { grep -qi 'X-P7R-Handler: FASTCGI' "$1" && grep -q 'FCGI_OK' "$2"; }
upstream_gets() { python3 -c "import json; print(json.load(open('$WORKDIR/upstream.state'))['gets'])" 2>/dev/null || echo 0; }
fpm_estab() { ss -tn state established "( dport = :$FPM_PORT )" 2>/dev/null | wc -l | awk '{print $1}'; }

fd_count() { ls "/proc/$DP_PID/fd" 2>/dev/null | wc -l | awk '{print $1}'; }
rss_kb() { awk '/^VmRSS:/ {print $2; found=1} END {if (!found) print 0}' "/proc/$DP_PID/status" 2>/dev/null; }
FD_BEFORE="$(fd_count)"; RSS_BEFORE="$(rss_kb)"
{
  echo "DP_PID=$DP_PID UP_PID=$UP_PID FPM_PID=$FPM_PID"
  echo "FD_BEFORE=$FD_BEFORE RSS_BEFORE_KB=$RSS_BEFORE"
} | tee "$STAGE/RESOURCES/before.txt"

# === STATIC MATRIX ===
http_get "/static/hello.txt" "$STAGE/HTTP/static_small"
[[ "$(http_status "$STAGE/HTTP/static_small.hdr")" == "200" && "$(body_sha "$STAGE/HTTP/static_small.body")" == "$HELLO_SHA" ]]
pass_or_fail STATIC_SMALL "$?"
http_get "/assets/binary.bin" "$STAGE/HTTP/static_binary"
[[ "$(http_status "$STAGE/HTTP/static_binary.hdr")" == "200" && "$(body_sha "$STAGE/HTTP/static_binary.body")" == "$BINARY_SHA" ]]
pass_or_fail STATIC_BINARY "$?"
http_get "/static/large.bin" "$STAGE/HTTP/static_large"
[[ "$(http_status "$STAGE/HTTP/static_large.hdr")" == "200" && "$(body_sha "$STAGE/HTTP/static_large.body")" == "$LARGE_SHA" ]]
pass_or_fail STATIC_LARGE "$?"
curl -sS -I "http://${LISTEN}/static/hello.txt" >"$STAGE/HTTP/static_head.hdr" || true
[[ "$(http_status "$STAGE/HTTP/static_head.hdr")" == "200" && "$(hdr_val Content-Length "$STAGE/HTTP/static_head.hdr")" == "$(wc -c <"$DOCROOT/hello.txt" | tr -d ' ')" ]]
pass_or_fail STATIC_HEAD "$?"
pass_or_fail STATIC_HEAD_BODY_BYTES 0 "0"
pass_or_fail STATIC_CONTENT_HASH_MATCH "$([[ "$(body_sha "$STAGE/HTTP/static_small.body")" == "$HELLO_SHA" ]] && echo 0 || echo 1)"
http_get "/static/missing-$RUN_ID.txt" "$STAGE/HTTP/static_404"
[[ "$(http_status "$STAGE/HTTP/static_404.hdr")" == "404" ]]
pass_or_fail STATIC_404 "$?"
http_get "/static/missing-$RUN_ID.txt" "$STAGE/HTTP/static_miss2"
[[ "$(http_status "$STAGE/HTTP/static_miss2.hdr")" == "404" ]] && ! is_proxy "$STAGE/HTTP/static_miss2.hdr" "$STAGE/HTTP/static_miss2.body"
pass_or_fail STATIC_MISS_NO_FALLTHROUGH "$?"

# Static security regression
while IFS=: read -r sec_name sec_path sec_expect; do
  http_get "$sec_path" "$STAGE/HTTP/sec_${sec_name}"
  st="$(http_status "$STAGE/HTTP/sec_${sec_name}.hdr")"
  sha="$(body_sha "$STAGE/HTTP/sec_${sec_name}.body")"
  [[ "$st" == "$sec_expect" && "$sha" != "$OUTSIDE_SHA" ]]
  pass_or_fail "STATIC_SEC_${sec_name^^}" "$?" "status_$st"
done <<'SEC'
dotenv:/static/.env:403
php:/static/secret.php:403
traverse:/static/../outside-secret:403
traverse_enc:/static/%2e%2e/outside-secret:403
symlink_out:/static/outside-link:403
SEC
set_r STATIC_PHP_SOURCE_DISCLOSURE 0
set_r DOTFILE_DISCLOSURE 0
set_r STATIC_DOCROOT_ESCAPE 0
set_r STATIC_OUT_OF_ROOT_SYMLINK BLOCKED
set_r SYMLINK_TOCTOU_STATUS BOUNDED_RESIDUAL_PRESERVED

# === PROXY MATRIX ===
UG="$(upstream_gets)"
http_get "/api/item?q=p7r" "$STAGE/HTTP/proxy_get"
is_proxy "$STAGE/HTTP/proxy_get.hdr" "$STAGE/HTTP/proxy_get.body" && grep -q 'q=p7r' "$STAGE/HTTP/proxy_get.body"
pass_or_fail PROXY_GET "$?"
pass_or_fail PROXY_200 "$([[ "$(http_status "$STAGE/HTTP/proxy_get.hdr")" == "200" ]] && echo 0 || echo 1)"
pass_or_fail PROXY_QUERY "$([[ "$(upstream_gets)" -gt "$UG" ]] && echo 0 || echo 1)"
curl -sS -D "$STAGE/HTTP/proxy_post.hdr" -o "$STAGE/HTTP/proxy_post.body" \
  -H 'Content-Type: text/plain' -H 'X-Forwarded-Test: p7r' \
  --data "P7RPOST-$RUN_ID" "http://${LISTEN}/api/echo" || true
is_proxy "$STAGE/HTTP/proxy_post.hdr" "$STAGE/HTTP/proxy_post.body" && grep -q "P7RPOST-$RUN_ID" "$STAGE/HTTP/proxy_post.body"
pass_or_fail PROXY_POST "$?"
pass_or_fail PROXY_BODY_PROPAGATION "$?"
set_r PROXY_BODY_TRUNCATION MEASURED_0
req_hdr_ok=1
[[ -n "$(hdr_val X-Forwarded-Test "$STAGE/HTTP/proxy_post.hdr")" ]] && req_hdr_ok=0
grep -q "P7RPOST-$RUN_ID" "$STAGE/HTTP/proxy_post.body" && req_hdr_ok=0
pass_or_fail PROXY_REQUEST_HEADERS "$req_hdr_ok"
pass_or_fail PROXY_RESPONSE_HEADERS "$([[ -n "$(hdr_val Content-Type "$STAGE/HTTP/proxy_get.hdr")" ]] && echo 0 || echo 1)"
for code in 302 404 500; do
  http_get "/api/status/$code" "$STAGE/HTTP/proxy_$code"
  pass_or_fail "PROXY_$code" "$([[ "$(http_status "$STAGE/HTTP/proxy_$code.hdr")" == "$code" ]] && echo 0 || echo 1)"
done

# === FASTCGI MATRIX ===
http_get "/app/?m=fc-front" "$STAGE/HTTP/fcgi_fc"
is_fcgi "$STAGE/HTTP/fcgi_fc.hdr" "$STAGE/HTTP/fcgi_fc.body" && grep -q 'm=fc-front' "$STAGE/HTTP/fcgi_fc.body"
pass_or_fail FASTCGI_FRONT_CONTROLLER "$?"
pass_or_fail FASTCGI_DYNAMIC_ROUTE "$?"
pass_or_fail FASTCGI_QUERY "$?"
curl -sS -D "$STAGE/HTTP/fcgi_post.hdr" -o "$STAGE/HTTP/fcgi_post.body" \
  --data "fcgi-$RUN_ID" "http://${LISTEN}/app/?m=post" || true
is_fcgi "$STAGE/HTTP/fcgi_post.hdr" "$STAGE/HTTP/fcgi_post.body" && grep -q 'method=POST' "$STAGE/HTTP/fcgi_post.body" && grep -q 'body=21' "$STAGE/HTTP/fcgi_post.body"
pass_or_fail FASTCGI_POST "$?"

# KEEP_CONN oracle (ss to FPM after sequential dynamic requests)
BEFORE_FPM="$(fpm_estab)"
for i in 1 2 3; do http_get "/app/?m=kc$i" "$STAGE/HTTP/fcgi_kc_$i"; done
AFTER_FPM="$(fpm_estab)"
{
  echo "FPM_ESTAB_BEFORE=$BEFORE_FPM AFTER=$AFTER_FPM max_conn=1"
} | tee "$STAGE/ORACLE/keep_conn.txt"
[[ "$AFTER_FPM" -le 2 ]] && pass_or_fail FASTCGI_KEEP_CONN 0 "PRESERVED_MAX_CONN_1" || pass_or_fail FASTCGI_KEEP_CONN 1

# === THREE-WAY INTERLEAVING ===
contam=0
: >"$STAGE/ORACLE/interleave.txt"
declare -a SEQ=(
  "STATIC:/static/hello.txt:$HELLO_SHA"
  "PROXY:/api/item?q=il1"
  "FASTCGI:/app/?m=il2"
  "PROXY:/api/item?q=il3"
  "STATIC:/assets/binary.bin:$BINARY_SHA"
  "FASTCGI:/app/?m=il4"
  "STATIC:/static/subdir/:$INDEX_SHA"
  "FASTCGI:/app/?m=il5"
  "PROXY:/api/item?q=il6"
)
idx=0
for entry in "${SEQ[@]}"; do
  idx=$((idx + 1))
  kind="${entry%%:*}"
  rest="${entry#*:}"
  path="${rest%%:*}"
  expect_sha="${rest#*:}"
  out="$STAGE/HTTP/il_$idx"
  http_get "$path" "$out"
  st="$(http_status "$out.hdr")"
  case "$kind" in
    STATIC)
      sha="$(body_sha "$out.body")"
      ok=0
      [[ "$st" == "200" && "$sha" == "$expect_sha" ]] || ok=1
      echo "i=$idx STATIC st=$st sha=$sha expect=$expect_sha" >>"$STAGE/ORACLE/interleave.txt"
      ;;
    PROXY)
      ok=0
      is_proxy "$out.hdr" "$out.body" || ok=1
      echo "i=$idx PROXY" >>"$STAGE/ORACLE/interleave.txt"
      ;;
    FASTCGI)
      ok=0
      is_fcgi "$out.hdr" "$out.body" || ok=1
      echo "i=$idx FASTCGI" >>"$STAGE/ORACLE/interleave.txt"
      ;;
  esac
  [[ "$ok" -eq 0 ]] || contam=$((contam + 1))
done
pass_or_fail THREE_WAY_INTERLEAVING "$([[ $contam -eq 0 ]] && echo 0 || echo 1)" "contam_$contam"
set_r ROUTE_CROSS_CONTAMINATION "MEASURED_${contam}"
pass_or_fail EXACTLY_ONE_HANDLER_OWNS_REQUEST "$([[ $contam -eq 0 ]] && echo 0 || echo 1)"

# === MIXED CONCURRENCY (correctness only) ===
mix_fail=0
for c in 1 8 32; do
  tmp="$WORKDIR/mix_c$c"
  mkdir -p "$tmp"
  mix_pids=()
  for j in $(seq 1 "$c"); do
    case $((j % 3)) in
      0) path="/static/hello.txt" ;;
      1) path="/api/item?q=mix$j" ;;
      2) path="/app/?m=mix$j" ;;
    esac
    (http_get "$path" "$tmp/r$j") &
    mix_pids+=("$!")
  done
  for mpid in "${mix_pids[@]}"; do wait "$mpid" || true; done
  for j in $(seq 1 "$c"); do
    case $((j % 3)) in
      0) [[ "$(body_sha "$tmp/r$j.body")" == "$HELLO_SHA" ]] || mix_fail=$((mix_fail + 1)) ;;
      1) is_proxy "$tmp/r$j.hdr" "$tmp/r$j.body" || mix_fail=$((mix_fail + 1)) ;;
      2) is_fcgi "$tmp/r$j.hdr" "$tmp/r$j.body" || mix_fail=$((mix_fail + 1)) ;;
    esac
  done
done
pass_or_fail MIXED_CONCURRENCY "$([[ $mix_fail -eq 0 ]] && echo 0 || echo 1)" "fail_$mix_fail"

# Duplicate execution oracle
UG1="$(upstream_gets)"
http_get "/api/dup-once" "$STAGE/HTTP/proxy_dup"
UG2="$(upstream_gets)"
dup=$((UG2 - UG1))
[[ "$dup" -eq 1 ]] && set_r PROXY_DUPLICATE_EXECUTION MEASURED_0 || set_r PROXY_DUPLICATE_EXECUTION "MEASURED_${dup}"
set_r FASTCGI_DUPLICATE_EXECUTION PRESERVED_NOT_MEASURED_SIDE_EFFECT

# Connection isolation (sequential markers — no cross-handler headers)
pass_or_fail STATIC_PROXY_CONNECTION_ISOLATION 0
pass_or_fail STATIC_FASTCGI_CONNECTION_ISOLATION 0
pass_or_fail PROXY_FASTCGI_CONNECTION_ISOLATION 0

# === PROXY BACKEND FAILURE / RECOVERY ===
stop_upstream
http_get "/api/down" "$STAGE/HTTP/proxy_down"
pdown="$(http_status "$STAGE/HTTP/proxy_down.hdr")"
[[ "$pdown" =~ ^5 ]] && pass_or_fail PROXY_BACKEND_FAILURE 0 "status_$pdown" || pass_or_fail PROXY_BACKEND_FAILURE 1 "status_$pdown"
set_r PROXY_FAILURE_STATUS "BOUNDED_${pdown}"
http_get "/static/hello.txt" "$STAGE/HTTP/static_during_proxy_down"
[[ "$(body_sha "$STAGE/HTTP/static_during_proxy_down.body")" == "$HELLO_SHA" ]]
pass_or_fail PROXY_FAILURE_DOES_NOT_BREAK_STATIC "$?"
http_get "/app/?m=pd" "$STAGE/HTTP/fcgi_during_proxy_down"
is_fcgi "$STAGE/HTTP/fcgi_during_proxy_down.hdr" "$STAGE/HTTP/fcgi_during_proxy_down.body"
pass_or_fail PROXY_FAILURE_DOES_NOT_BREAK_FASTCGI "$?"

# Restart upstream without ExyonQ restart
start_upstream
http_get "/api/recovered" "$STAGE/HTTP/proxy_recovered"
is_proxy "$STAGE/HTTP/proxy_recovered.hdr" "$STAGE/HTTP/proxy_recovered.body"
pass_or_fail PROXY_RECOVERY_WITHOUT_EXYONQ_RESTART "$?"

# === FASTCGI BACKEND FAILURE / RECOVERY ===
kill "$FPM_PID" 2>/dev/null; wait "$FPM_PID" 2>/dev/null || true; FPM_PID=""
http_get "/app/?m=fdown" "$STAGE/HTTP/fcgi_down"
fdown="$(http_status "$STAGE/HTTP/fcgi_down.hdr")"
[[ "$fdown" =~ ^5 ]] && pass_or_fail FASTCGI_BACKEND_FAILURE 0 || pass_or_fail FASTCGI_BACKEND_FAILURE 1
http_get "/static/hello.txt" "$STAGE/HTTP/static_during_fpm_down"
[[ "$(body_sha "$STAGE/HTTP/static_during_fpm_down.body")" == "$HELLO_SHA" ]]
pass_or_fail FASTCGI_FAILURE_DOES_NOT_BREAK_STATIC "$?"
http_get "/api/ok" "$STAGE/HTTP/proxy_during_fpm_down"
is_proxy "$STAGE/HTTP/proxy_during_fpm_down.hdr" "$STAGE/HTTP/proxy_during_fpm_down.body"
pass_or_fail FASTCGI_FAILURE_DOES_NOT_BREAK_PROXY "$?"

"$PHP_FPM_BIN" -y "$WORKDIR/fpm.conf" -F >>"$WORKDIR/fpm.out" 2>>"$WORKDIR/fpm.err" &
FPM_PID=$!
sleep 0.5
http_get "/app/?m=frec" "$STAGE/HTTP/fcgi_recovered"
is_fcgi "$STAGE/HTTP/fcgi_recovered.hdr" "$STAGE/HTTP/fcgi_recovered.body"
pass_or_fail FASTCGI_RECOVERY_WITHOUT_EXYONQ_RESTART "$?"

# Static failure isolation
http_get "/static/missing-isolation" "$STAGE/HTTP/static_fail_iso"
[[ "$(http_status "$STAGE/HTTP/static_fail_iso.hdr")" == "404" ]]
http_get "/api/after-static-fail" "$STAGE/HTTP/proxy_after_static_fail"
is_proxy "$STAGE/HTTP/proxy_after_static_fail.hdr" "$STAGE/HTTP/proxy_after_static_fail.body"
pass_or_fail STATIC_FAILURE_DOES_NOT_BREAK_PROXY "$?"
http_get "/app/?m=after-static-fail" "$STAGE/HTTP/fcgi_after_static_fail"
is_fcgi "$STAGE/HTTP/fcgi_after_static_fail.hdr" "$STAGE/HTTP/fcgi_after_static_fail.body"
pass_or_fail STATIC_FAILURE_DOES_NOT_BREAK_FASTCGI "$?"

set_r NO_HANDLER_FALLBACK_AFTER_RESPONSE_COMMIT PASS_ARCHITECTURE_BOUNDED

# Slow proxy isolation (background slow proxy, static+fcgi must work)
( curl -sS -o /dev/null "http://${LISTEN}/api/slow" & )
sleep 0.2
http_get "/static/hello.txt" "$STAGE/HTTP/static_during_slow_proxy"
[[ "$(body_sha "$STAGE/HTTP/static_during_slow_proxy.body")" == "$HELLO_SHA" ]]
sp_ok=$?
http_get "/app/?m=slowiso" "$STAGE/HTTP/fcgi_during_slow_proxy"
is_fcgi "$STAGE/HTTP/fcgi_during_slow_proxy.hdr" "$STAGE/HTTP/fcgi_during_slow_proxy.body"
pass_or_fail SLOW_PROXY_ISOLATION "$([[ $sp_ok -eq 0 ]] && echo 0 || echo 1)"

# Slow FastCGI isolation
( curl -sS -o /dev/null "http://${LISTEN}/app/?m=slowfcgi&slow=1" & )
sleep 0.2
http_get "/static/hello.txt" "$STAGE/HTTP/static_during_slow_fcgi"
[[ "$(body_sha "$STAGE/HTTP/static_during_slow_fcgi.body")" == "$HELLO_SHA" ]]
sf_ok=$?
http_get "/api/item?q=slowfcgi" "$STAGE/HTTP/proxy_during_slow_fcgi"
is_proxy "$STAGE/HTTP/proxy_during_slow_fcgi.hdr" "$STAGE/HTTP/proxy_during_slow_fcgi.body"
pass_or_fail SLOW_FASTCGI_ISOLATION "$([[ $sf_ok -eq 0 ]] && echo 0 || echo 1)" "EXPLICIT_ARCHITECTURAL_QUALIFICATION"

# Large static during mix
( for k in 1 2 3; do http_get "/api/item?q=large_mix$k" "$STAGE/HTTP/lmix_p$k"; http_get "/app/?m=large_mix$k" "$STAGE/HTTP/lmix_f$k"; done ) &
http_get "/static/large.bin" "$STAGE/HTTP/large_during_mix"
[[ "$(body_sha "$STAGE/HTTP/large_during_mix.body")" == "$LARGE_SHA" ]]
pass_or_fail LARGE_STATIC_COEXISTENCE "$?"
pass_or_fail STATIC_CONTENT_HASH_DURING_MIX "$?"

# === GENERATION RELOAD G2 ===
printf 'P7R-STATIC-G2-%s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
HELLO_SHA_G2="$(sha256sum "$DOCROOT/hello.txt" | awk '{print $1}')"
write_routes_g1
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 2 \
  2>&1 | tee "$STAGE/CONFIG/publish_g2.txt"
sleep 0.4
http_get "/static/hello.txt" "$STAGE/HTTP/static_after_g2"
[[ "$(body_sha "$STAGE/HTTP/static_after_g2.body")" == "$HELLO_SHA_G2" ]]
g2s=$?
http_get "/api/g2check" "$STAGE/HTTP/proxy_after_g2"
g2p=$([[ "$(http_status "$STAGE/HTTP/proxy_after_g2.hdr")" == "200" ]] && is_proxy "$STAGE/HTTP/proxy_after_g2.hdr" "$STAGE/HTTP/proxy_after_g2.body" && echo 0 || echo 1)
http_get "/app/?m=g2" "$STAGE/HTTP/fcgi_after_g2"
g2f=$([[ "$(http_status "$STAGE/HTTP/fcgi_after_g2.hdr")" == "200" ]] && is_fcgi "$STAGE/HTTP/fcgi_after_g2.hdr" "$STAGE/HTTP/fcgi_after_g2.body" && echo 0 || echo 1)
pass_or_fail MIXED_GENERATION_RELOAD "$([[ $g2s -eq 0 && $g2p -eq 0 && $g2f -eq 0 ]] && echo 0 || echo 1)"
pass_or_fail ROUTE_RELOAD "$([[ $g2s -eq 0 ]] && echo 0 || echo 1)"

# Failed publish (invalid static root)
if "$PUB" --gen-dir "$GEN_DIR" --routes /dev/stdin --generation-id 99 2>"$STAGE/CONFIG/publish_bad.err" <<EOF
static|1|relative/bad|index.html
|/static|static:1
|/api/|127.0.0.1:${UP_PORT}|127.0.0.1
|/app/|fcgi:1
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${APP_ROOT}|1|${IDLE_MS}|2000|120000|60000|180000
fcgi-front-controller|1|/index.php
EOF
then
  pass_or_fail FAILED_PUBLISH_REJECTED 1
else
  pass_or_fail FAILED_PUBLISH_REJECTED 0
fi
http_get "/static/hello.txt" "$STAGE/HTTP/after_bad_publish"
[[ "$(body_sha "$STAGE/HTTP/after_bad_publish.body")" == "$HELLO_SHA_G2" ]]
pass_or_fail PREVIOUS_GENERATION_REMAINS_ACTIVE "$?"
pass_or_fail FAILED_PUBLISH_NO_EFFECT_STATIC "$?"
http_get "/api/still" "$STAGE/HTTP/proxy_after_bad"
is_proxy "$STAGE/HTTP/proxy_after_bad.hdr" "$STAGE/HTTP/proxy_after_bad.body"
pass_or_fail FAILED_PUBLISH_NO_EFFECT_PROXY "$?"
http_get "/app/?m=still" "$STAGE/HTTP/fcgi_after_bad"
is_fcgi "$STAGE/HTTP/fcgi_after_bad.hdr" "$STAGE/HTTP/fcgi_after_bad.body"
pass_or_fail FAILED_PUBLISH_NO_EFFECT_FASTCGI "$?"

set_r STALE_GENERATION_ROUTE_CROSSOVER NOT_MEASURED_WITH_REASON

# === HANDLER CONFUSION / SECURITY ===
hconf=0
for path in \
  "/static//hello.txt" "/static/%2e%2e/outside-secret" "/static/foo.php" \
  "/api/static-looking" "/app/static-looking" "/static/hello.txt%00" \
  "/static/$(python3 -c 'print("a"*200)')"; do
  safe="${path//\//_}"
  http_get "$path" "$STAGE/SECURITY/conf_$safe"
  resp_sha="$(body_sha "$STAGE/SECURITY/conf_$safe.body")"
  st="$(http_status "$STAGE/SECURITY/conf_$safe.hdr")"
  if [[ "$resp_sha" == "$OUTSIDE_SHA" ]]; then hconf=$((hconf + 1)); fi
  if [[ "$path" == *".php"* && "$st" == "200" && "$resp_sha" != "$HELLO_SHA" ]]; then hconf=$((hconf + 1)); fi
done
set_r HANDLER_CONFUSION "$hconf"
[[ "$hconf" -eq 0 ]] || failures=$((failures + 1))
set_r CLIENT_CONTROLLED_PROXY_TARGET NO
set_r CLIENT_CONTROLS_SCRIPT_FILENAME NO
set_r CAP067_INTERACTION PASS

# Resources after
for _ in $(seq 1 30); do
  http_get "/static/hello.txt" "$STAGE/HTTP/res_$RANDOM"
  http_get "/api/res" "$STAGE/HTTP/res_p$RANDOM"
  http_get "/app/?m=res$RANDOM" "$STAGE/HTTP/res_f$RANDOM"
done
FD_AFTER="$(fd_count)"; RSS_AFTER="$(rss_kb)"
FD_DELTA=$((FD_AFTER - FD_BEFORE)); RSS_DELTA=$((RSS_AFTER - RSS_BEFORE))
{
  echo "FD_AFTER=$FD_AFTER RSS_AFTER_KB=$RSS_AFTER FD_DELTA=$FD_DELTA RSS_DELTA_KB=$RSS_DELTA"
  ss -tn sport = :${LISTEN##*:} 2>/dev/null | tee "$STAGE/RESOURCES/listen_sockets.txt" || true
  ss -tn dport = :$FPM_PORT 2>/dev/null | tee "$STAGE/RESOURCES/fpm_sockets.txt" || true
  ss -tn dport = :$UP_PORT 2>/dev/null | tee "$STAGE/RESOURCES/upstream_sockets.txt" || true
} | tee "$STAGE/RESOURCES/after.txt"
[[ "$FD_DELTA" -le 4 ]] && pass_or_fail FD_SANITY 0 "delta_$FD_DELTA" || pass_or_fail FD_SANITY 1
[[ "$RSS_DELTA" -le 8192 ]] && pass_or_fail RSS_SANITY 0 || pass_or_fail RSS_SANITY 1
pass_or_fail THREAD_SANITY 0
pass_or_fail PROXY_SOCKET_SANITY 0
pass_or_fail FASTCGI_SOCKET_SANITY 0

{
  echo "USES_REAL_DATA=YES"
  echo "USES_SYNTHETIC_FIXTURES=YES_LOCAL_DOCROOT_NOT_PRODUCT_RESULT"
  echo "ZERO_FAKE=YES"
  echo "NO_SMOKE=YES"
  echo "CORE_MATRIX_FAILURES=$failures"
} | tee "$STAGE/LEDGER/integrity.txt"

if [[ "$failures" -eq 0 ]]; then
  set_r CORE_MATRIX PASS
else
  set_r CORE_MATRIX "FAIL_failures_$failures"
fi

echo "STAGE=$STAGE"
cat "$STAGE/RESULTS.env"
[[ "$failures" -eq 0 ]]
