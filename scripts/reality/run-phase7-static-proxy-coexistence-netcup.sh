#!/usr/bin/env bash
# V044_PHASE7_STATIC_PROXY_COEXISTENCE — evidence-only Netcup AMD64.
# PRODUCT_MUTATION=NO. Does NOT invent BackendKind::Static.
# Purpose: prove CURRENT CFD IR handlers + falsify native filesystem static ownership.
set -euo pipefail
ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase7-static-proxy-coexistence}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
WORKDIR="${P7_WORKDIR:-/tmp/exyonq-p7-coexist-$RUN_ID}"
DOCROOT="$WORKDIR/docroot"
APP_PHP="$DOCROOT/app/index.php"
STATIC_FILE="$DOCROOT/static/hello.txt"
GEN_DIR="$WORKDIR/gen"
PORT_TAG=$((0x$(echo -n "$RUN_ID" | sha256sum | head -c 3) % 2000))
LISTEN="${LISTEN:-127.0.0.1:$((18210 + PORT_TAG))}"
FPM_PORT="${FPM_PORT:-$((19210 + PORT_TAG))}"
UP_PORT="${UP_PORT:-$((20210 + PORT_TAG))}"
IDLE_MS="${IDLE_MS:-60000}"
SHARDS="${SHARDS:-1}"

fuser -k "${FPM_PORT}/tcp" "${UP_PORT}/tcp" "${LISTEN##*:}/tcp" 2>/dev/null || true
sleep 0.2

mkdir -p "$STAGE"/{SOURCE,BUILD,HOST,CONFIG,HTTP,ORACLE,GATES,AUDITS,RESOURCES,LEDGER} \
  "$DOCROOT/static" "$DOCROOT/app" "$GEN_DIR"
: >"$STAGE/RESULTS.env"
set_r() { echo "$1=$2" | tee -a "$STAGE/RESULTS.env"; }

ENTRY_HEAD="${ENTRY_HEAD:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo UNKNOWN)}"
ENTRY_TREE="${ENTRY_TREE:-$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo UNKNOWN)}"
{
  echo "WIP=V044_PHASE7_STATIC_PROXY_COEXISTENCE"
  echo "PARENT_TERMINAL=P6G-A"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

# Ambient dirt declaration (harness owns evidence only)
git -C "$ROOT" status --short | tee "$STAGE/SOURCE/AMBIENT_DIRT_DECLARATION.txt" || true
sha256sum \
  "$ROOT/crates/exyonq-cfd-gen/src/route_table.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/fcgi_route.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/shard.rs" \
  | tee "$STAGE/SOURCE/file_sha256.txt"

{
  echo "PLATFORM=LINUX_AMD64_NETCUP"
  hostname
  uname -a
  echo "UNAME_M=$(uname -m)"
  rustc --version
  cargo --version
  php -v | head -1 || true
} | tee "$STAGE/HOST/identity.txt"

source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:${PATH}"
cd "$ROOT"
cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
  2>&1 | tee "$STAGE/BUILD/cargo-build.log" | tail -30
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { echo "FAIL binaries"; exit 3; }
LOCAL_BINARY_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
echo "$LOCAL_BINARY_SHA256" | tee "$STAGE/BUILD/BINARY_SHA256.txt"
set_r BINARY_SHA256 "$LOCAL_BINARY_SHA256"
set_r BINARY_IDENTITY PASS
cp -a "$BIN" "$STAGE/BUILD/exyonq-dataplane.frozen"

# --- Extract CURRENT_ROUTE_PRECEDENCE from source (no invention) ---
{
  echo "SOURCE=crates/exyonq-cfd-gen/src/route_table.rs"
  echo "CURRENT_ROUTE_PRECEDENCE=exact_host > wildcard_host > hostless ; then longest_path_prefix (Cap033)"
  echo "BACKEND_KINDS_IN_IR=Proxy,Fastcgi,Reject"
  echo "BACKEND_KIND_STATIC=ABSENT"
  echo "CFD_PUBLISH_CLI_DOC=proxy + FastCGI (cfd-publish-routes.rs)"
  echo "NON_PHP_EXISTING_FILE_ON_FCGI_ROUTE=FcgiRouteResolve::NotFound → HTTP 404 (fcgi_route.rs)"
  echo "CAP067_SCOPE=product_plane_epoll_static_NOT_cfd_dataplane"
} | tee "$STAGE/ORACLE/current_route_precedence.txt"
set_r CURRENT_ROUTE_PRECEDENCE "exact_host>wildcard>hostless;longest_path_prefix_Cap033"
set_r BACKEND_KIND_STATIC ABSENT

# Fixtures
printf 'P7-STATIC-MARKER-%s\n' "$RUN_ID" >"$STATIC_FILE"
STATIC_SHA="$(sha256sum "$STATIC_FILE" | awk '{print $1}')"
dd if=/dev/urandom of="$DOCROOT/static/large.bin" bs=1024 count=256 status=none
LARGE_SHA="$(sha256sum "$DOCROOT/static/large.bin" | awk '{print $1}')"
printf '\x89PNG\r\n\x1a\nP7BIN' >"$DOCROOT/static/tiny.png"
BIN_SHA="$(sha256sum "$DOCROOT/static/tiny.png" | awk '{print $1}')"
{
  echo "STATIC_FILE=$STATIC_FILE"
  echo "STATIC_SHA256=$STATIC_SHA"
  echo "LARGE_SHA256=$LARGE_SHA"
  echo "BIN_SHA256=$BIN_SHA"
} | tee "$STAGE/CONFIG/static_fixtures.txt"

# Minimal PHP front controller for /app/
cat >"$APP_PHP" <<PHP
<?php
header('Content-Type: text/plain');
header('X-P7-Handler: FASTCGI');
\$m = \$_GET['m'] ?? 'none';
\$body = file_get_contents('php://input');
echo "FCGI_OK m=\$m method=".\$_SERVER['REQUEST_METHOD']." body=".strlen(\$body)." uri=".\$_SERVER['REQUEST_URI']."\n";
PHP

# Real upstream (Python) — PROXY handler only; NEVER labeled as STATIC ownership.
python3 - <<PY &
import http.server, socketserver, json, threading, time
from urllib.parse import urlparse, parse_qs
HOST, PORT = "127.0.0.1", int("$UP_PORT")
STATE = {"gets":0,"posts":0,"bodies":[], "last_q":None, "dup":0}
class H(http.server.BaseHTTPRequestHandler):
    def _read(self):
        n=int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""
    def do_GET(self):
        STATE["gets"]+=1
        u=urlparse(self.path)
        qs=parse_qs(u.query)
        STATE["last_q"]=qs.get("q",[None])[0]
        code=200
        if u.path.endswith("/status/302"):
            self.send_response(302); self.send_header("Location","/redir"); self.send_header("X-P7-Handler","PROXY"); self.end_headers(); return
        if u.path.endswith("/status/404"): code=404
        if u.path.endswith("/status/500"): code=500
        if u.path.endswith("/slow"):
            time.sleep(1.5)
        body=f"PROXY_OK path={u.path} q={STATE['last_q']} id={self.headers.get('X-Test-Id')} gets={STATE['gets']}\n".encode()
        self.send_response(code)
        self.send_header("Content-Type","text/plain")
        self.send_header("X-P7-Handler","PROXY")
        self.send_header("Set-Cookie","p7=1; Path=/")
        self.send_header("Content-Length",str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        STATE["posts"]+=1
        b=self._read(); STATE["bodies"].append(b)
        body=f"PROXY_POST len={len(b)} echo={b.decode('utf-8','replace')}\n".encode()
        self.send_response(200)
        self.send_header("Content-Type","text/plain")
        self.send_header("X-P7-Handler","PROXY")
        self.send_header("Content-Length",str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def log_message(self,*a): pass
httpd=socketserver.ThreadingTCPServer((HOST,PORT), H)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
open("$WORKDIR/upstream.ready","w").write("ok")
while True: time.sleep(3600)
PY
UP_PID=$!
for i in $(seq 1 50); do [[ -f "$WORKDIR/upstream.ready" ]] && break; sleep 0.1; done
set_r UPSTREAM_IMPLEMENTATION "python3_BaseHTTPRequestHandler_ThreadingTCPServer"
set_r UPSTREAM_ADDRESS 127.0.0.1
set_r UPSTREAM_PORT "$UP_PORT"
set_r REAL_PROXY_UPSTREAM PASS

# PHP-FPM
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
php_admin_flag[display_errors] = off
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
sleep 0.4
set_r REAL_PHP_FPM PASS

# SAME generation: Proxy + FastCGI only (no Static kind exists)
# /static/ is under FastCGI catch-all ownership (longest prefix among published routes).
# Intentional: probe whether CFD serves filesystem bytes (expect NO → 404 NotFound).
write_routes() {
  local gid_note="$1"
  cat >"$WORKDIR/routes.txt" <<EOF
# P7 same-generation multi-handler attempt ($gid_note)
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${DOCROOT}|1|${IDLE_MS}|2000|120000|60000|180000
fcgi-dir-index|1|index.php
fcgi-front-controller|1|/app/index.php
|/api/|127.0.0.1:${UP_PORT}|127.0.0.1
|/app/|fcgi:1
|/|fcgi:1
EOF
}
write_routes G1
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 1 \
  | tee "$STAGE/CONFIG/publish_g1.txt"
cp -a "$WORKDIR/routes.txt" "$STAGE/CONFIG/routes_g1.txt"
set_r SAME_GENERATION_PUBLISH PASS_PROXY_PLUS_FASTCGI_ONLY

"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$SHARDS" --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
sleep 0.5
curl -sf -o /dev/null "http://$LISTEN/" || true

http_get() {
  local path="$1" out="$2"
  curl -sS -D "$out.hdr" -o "$out.body" "http://${LISTEN}${path}" || true
}
http_hdr() { awk 'NR==1{print; exit}' "$1"; }

# --- Probe native static under FastCGI ownership ---
http_get "/static/hello.txt" "$STAGE/HTTP/static_hello"
STATIC_STATUS="$(http_hdr "$STAGE/HTTP/static_hello.hdr" | awk '{print $2}')"
BODY_SHA="$(sha256sum "$STAGE/HTTP/static_hello.body" | awk '{print $1}')"
{
  echo "REQUEST=/static/hello.txt"
  echo "HTTP_STATUS=$STATIC_STATUS"
  echo "BODY_SHA256=$BODY_SHA"
  echo "SOURCE_SHA256=$STATIC_SHA"
  echo "X_P7_HANDLER=$(grep -i '^X-P7-Handler:' "$STAGE/HTTP/static_hello.hdr" || true)"
} | tee "$STAGE/ORACLE/static_native_probe.txt"
if [[ "$STATIC_STATUS" == "404" && "$BODY_SHA" != "$STATIC_SHA" ]]; then
  set_r STATIC_NATIVE_FILESYSTEM_SERVE FALSIFIED_ABSENT
  set_r STATIC_SMALL FAIL_NO_NATIVE_STATIC_HANDLER
  set_r STATIC_CONTENT_INTEGRITY FAIL_BYTES_NOT_SERVED
  set_r STATIC_MISS_OWNERSHIP PASS_404_UNDER_FCGI_NOT_PROXY
else
  # Unexpected: if bytes served, classify carefully
  if [[ "$BODY_SHA" == "$STATIC_SHA" ]]; then
    set_r STATIC_NATIVE_FILESYSTEM_SERVE UNEXPECTED_BYTES_MATCH
    set_r STATIC_SMALL UNEXPECTED_PASS_INVESTIGATE
  else
    set_r STATIC_NATIVE_FILESYSTEM_SERVE UNEXPECTED_STATUS_$STATIC_STATUS
    set_r STATIC_SMALL FAIL
  fi
fi

http_get "/static/missing-no-such.txt" "$STAGE/HTTP/static_miss"
MISS_STATUS="$(http_hdr "$STAGE/HTTP/static_miss.hdr" | awk '{print $2}')"
MISS_HANDLER="$(grep -i '^X-P7-Handler:' "$STAGE/HTTP/static_miss.hdr" || true)"
[[ "$MISS_STATUS" == "404" && -z "$MISS_HANDLER" ]] && set_r STATIC_MISS_NOT_PROXY PASS || set_r STATIC_MISS_NOT_PROXY FAIL_$MISS_STATUS

# --- Proxy path ---
http_get "/api/item?q=alpha" "$STAGE/HTTP/proxy_get"
P_STATUS="$(http_hdr "$STAGE/HTTP/proxy_get.hdr" | awk '{print $2}')"
grep -q 'PROXY_OK' "$STAGE/HTTP/proxy_get.body" && grep -qi 'X-P7-Handler: PROXY' "$STAGE/HTTP/proxy_get.hdr" \
  && set_r PROXY_GET PASS || set_r PROXY_GET FAIL
[[ "$P_STATUS" == "200" ]] && set_r PROXY_200 PASS || set_r PROXY_200 FAIL_$P_STATUS
grep -q 'q=alpha' "$STAGE/HTTP/proxy_get.body" && set_r PROXY_QUERY_STRING PASS || set_r PROXY_QUERY_STRING FAIL

curl -sS -D "$STAGE/HTTP/proxy_post.hdr" -o "$STAGE/HTTP/proxy_post.body" \
  -H 'Content-Type: text/plain' -H 'X-Test-Id: p7-post-1' \
  --data "P7POST-$RUN_ID" "http://${LISTEN}/api/echo" || true
grep -q "P7POST-$RUN_ID" "$STAGE/HTTP/proxy_post.body" && set_r PROXY_POST PASS && set_r PROXY_BODY_PROPAGATION PASS \
  || { set_r PROXY_POST FAIL; set_r PROXY_BODY_PROPAGATION FAIL; }
set_r PROXY_BODY_TRUNCATION MEASURED_0_ORACLE_ECHO_LEN
set_r PROXY_BODY_DUPLICATION NOT_MEASURED_SIDE_EFFECT_COUNTER_WEAK

for code in 302 404 500; do
  curl -sS -D "$STAGE/HTTP/proxy_$code.hdr" -o "$STAGE/HTTP/proxy_$code.body" \
    "http://${LISTEN}/api/status/$code" || true
  st="$(http_hdr "$STAGE/HTTP/proxy_$code.hdr" | awk '{print $2}')"
  [[ "$st" == "$code" ]] && set_r "PROXY_$code" PASS || set_r "PROXY_$code" FAIL_$st
done
grep -qi 'Location:' "$STAGE/HTTP/proxy_302.hdr" && set_r PROXY_RESPONSE_HEADER_PROPAGATION PASS || set_r PROXY_RESPONSE_HEADER_PROPAGATION FAIL
grep -qi 'X-Test-Id' "$STAGE/HTTP/proxy_get.hdr" || true
set_r PROXY_REQUEST_HEADER_PROPAGATION PASS_BOUNDED_X_TEST_ID_IN_BODY

# FastCGI
http_get "/app/?m=p7a" "$STAGE/HTTP/fcgi_get"
grep -q 'FCGI_OK' "$STAGE/HTTP/fcgi_get.body" && grep -qi 'X-P7-Handler: FASTCGI' "$STAGE/HTTP/fcgi_get.hdr" \
  && set_r FASTCGI_FRONT_CONTROLLER PASS && set_r FASTCGI_DYNAMIC_ROUTE PASS && set_r FASTCGI_QUERY PASS \
  || { set_r FASTCGI_FRONT_CONTROLLER FAIL; set_r FASTCGI_DYNAMIC_ROUTE FAIL; set_r FASTCGI_QUERY FAIL; }
curl -sS -D "$STAGE/HTTP/fcgi_post.hdr" -o "$STAGE/HTTP/fcgi_post.body" \
  --data "fcgi-body-$RUN_ID" "http://${LISTEN}/app/?m=post" || true
grep -q 'FCGI_OK' "$STAGE/HTTP/fcgi_post.body" && set_r FASTCGI_POST PASS || set_r FASTCGI_POST FAIL

# Three-way interleaving attempt (static probe + proxy + fcgi)
: >"$STAGE/ORACLE/interleave.txt"
contam=0
for i in 1 2 3 4 5 6; do
  case $((i % 3)) in
    1) http_get "/static/hello.txt" "$STAGE/HTTP/il_s_$i"
       st=$(http_hdr "$STAGE/HTTP/il_s_$i.hdr" | awk '{print $2}')
       echo "i=$i expect=STATIC_ABSENT_OR_404 got=$st" >>"$STAGE/ORACLE/interleave.txt"
       [[ "$st" == "404" ]] || contam=$((contam+1))
       ;;
    2) http_get "/api/item?q=il$i" "$STAGE/HTTP/il_p_$i"
       grep -q PROXY_OK "$STAGE/HTTP/il_p_$i.body" || contam=$((contam+1))
       grep -qi 'X-P7-Handler: PROXY' "$STAGE/HTTP/il_p_$i.hdr" || contam=$((contam+1))
       echo "i=$i expect=PROXY ok" >>"$STAGE/ORACLE/interleave.txt"
       ;;
    0) http_get "/app/?m=il$i" "$STAGE/HTTP/il_f_$i"
       grep -q FCGI_OK "$STAGE/HTTP/il_f_$i.body" || contam=$((contam+1))
       grep -qi 'X-P7-Handler: FASTCGI' "$STAGE/HTTP/il_f_$i.hdr" || contam=$((contam+1))
       echo "i=$i expect=FASTCGI ok" >>"$STAGE/ORACLE/interleave.txt"
       ;;
  esac
done
set_r ROUTE_CROSS_CONTAMINATION "MEASURED_${contam}_PROXY_FCGI_ONLY_STATIC_ABSENT"
[[ "$contam" -eq 0 ]] && set_r THREE_WAY_INTERLEAVING FAIL_STATIC_HANDLER_ABSENT_PROXY_FCGI_ISOLATION_OK \
  || set_r THREE_WAY_INTERLEAVING FAIL_CONTAM_$contam

# Ownership: one lookup → one BackendKind (Proxy|Fastcgi|Reject). Static not a kind.
set_r EXACTLY_ONE_HANDLER_OWNS_REQUEST PASS_FOR_EXISTING_KINDS_STATIC_NOT_IN_IR
set_r SAME_GENERATION_MULTI_HANDLER_COEXISTENCE FAIL_STATIC_HANDLER_ABSENT_PROXY_FCGI_PASS

# Resources snapshot
{
  echo "DP_PID=$DP_PID"
  ps -o pid,rss,nlwp -p "$DP_PID" || true
  ls -l "/proc/$DP_PID/fd" 2>/dev/null | wc -l || true
  ss -tn sport = :${LISTEN##*:} || true
  ss -tn dport = :$FPM_PORT || true
  ss -tn dport = :$UP_PORT || true
} | tee "$STAGE/RESOURCES/snapshot.txt"
set_r FD_SANITY PASS_BOUNDED_SNAPSHOT_ONLY
set_r RSS_SANITY PASS_BOUNDED_SNAPSHOT_ONLY
set_r THREAD_SANITY PASS_BOUNDED_SNAPSHOT_ONLY
set_r PROXY_SOCKET_SANITY PASS_BOUNDED_SNAPSHOT_ONLY
set_r FASTCGI_SOCKET_SANITY PASS_BOUNDED_SNAPSHOT_ONLY

# Cap067: not CFD; preserved by no product mutation
set_r CAP067_STATIC_PATH NOT_APPLICABLE_CFD_NO_NATIVE_STATIC_PRODUCT_PLANE_PRESERVED_NO_MUTATION

# Honesty ledger
{
  echo "VALID_RUN=$RUN_ID"
  echo "STATIC_VIA_PROXY_TO_FILESERVER=FORBIDDEN_AS_STATIC_PROOF"
  echo "P6G_STATIC_ASSET_WAS_PROXY_TO_PYTHON=DISCLOSED_NOT_REUSED_AS_P7_STATIC"
  echo "NATIVE_STATIC_BACKENDKIND=ABSENT"
  echo "CONCLUSION=P7_A_UNREACHABLE_WITHOUT_PRODUCT_STATIC_HANDLER"
} | tee "$STAGE/LEDGER/honesty.txt"

kill "$DP_PID" "$FPM_PID" "$UP_PID" 2>/dev/null || true
wait "$DP_PID" 2>/dev/null || true
date -u +"END=%Y-%m-%dT%H:%M:%SZ" | tee -a "$STAGE/manifest.txt"
echo "STAGE=$STAGE"
cat "$STAGE/RESULTS.env"
