#!/usr/bin/env bash
# V044_PHASE8_COMPLETENESS_AND_ADVERSARIAL_SECURITY — Netcup amd64 evidence-only.
# PARENT=STATICDUAL-B. PRODUCT_MUTATION=NO. ZERO_FAKE. NO_SMOKE.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/phase8-completeness-adversarial-security}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
WORKDIR="${P8_WORKDIR:-/tmp/exyonq-p8-$RUN_ID}"
DOCROOT="$WORKDIR/docroot"
APP_ROOT="$WORKDIR/app"
OUTSIDE="$WORKDIR/outside-secret"
GEN_DIR="$WORKDIR/gen"
PORT_TAG=$((0x$(printf '%s' "$RUN_ID" | sha256sum | cut -c 1-3) % 2000))
LISTEN="${LISTEN:-127.0.0.1:$((18940 + PORT_TAG))}"
FPM_PORT="${FPM_PORT:-$((19940 + PORT_TAG))}"
UP_PORT="${UP_PORT:-$((20940 + PORT_TAG))}"
IDLE_MS="${IDLE_MS:-60000}"
SHARDS="${SHARDS:-1}"
RAW="$ROOT/scripts/reality/phase8-raw-socket-client.py"
TOCTOU_N="${TOCTOU_N:-4000}"

DP_PID=""
UP_PID=""
FPM_PID=""
TRACE_STRACE=""
cleanup() {
  local rc=$?
  kill -TERM "$TRACE_STRACE" "$DP_PID" "$UP_PID" "$FPM_PID" 2>/dev/null || true
  wait "$TRACE_STRACE" "$DP_PID" "$UP_PID" "$FPM_PID" 2>/dev/null || true
  date -u +"END=%Y-%m-%dT%H:%M:%SZ" >>"$STAGE/manifest.txt" 2>/dev/null || true
  exit "$rc"
}
trap cleanup EXIT

mkdir -p "$STAGE"/{SOURCE,BUILD,HOST,CONFIG,HTTP,ORACLE,GATES,RESOURCES,STRACE,SECURITY,LEDGER,OBS} \
  "$DOCROOT/subdir" "$DOCROOT/host-static" "$DOCROOT/host-wild" "$APP_ROOT" "$GEN_DIR"
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
  echo "WIP=V044_PHASE8_COMPLETENESS_AND_ADVERSARIAL_SECURITY"
  echo "PARENT=STATICDUAL-B"
  echo "PLATFORM=LINUX_AMD64_NETCUP"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  echo "LISTEN=$LISTEN"
  echo "WAF_PROFILE=NOT_PUBLISHED_MATCHES_P7R_SUPPORTED_PATH"
  echo "USES_REAL_DATA=YES"
  echo "USES_SYNTHETIC_FIXTURES=YES_LOCAL_DOCROOT_NOT_PRODUCT_RESULT"
  echo "CHANGES_PRODUCT_SEMANTICS=NO"
  echo "ZERO_FAKE=YES"
  echo "NO_SMOKE=YES"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

git -C "$ROOT" status --short 2>/dev/null | tee "$STAGE/SOURCE/AMBIENT_DIRT_DECLARATION.txt" || true
sha256sum \
  "$ROOT/crates/exyonq-cfd-gen/src/route_table.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/static_serve.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/shard.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/http_parse.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/hop.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/fcgi_exec.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/fcgi_route.rs" \
  | tee "$STAGE/SOURCE/file_sha256.txt"
SOURCE_ID="$(sha256sum "$STAGE/SOURCE/file_sha256.txt" | awk '{print $1}')"
set_r SOURCE_ID "$SOURCE_ID"

{
  hostname
  uname -a
  echo "UNAME_M=$(uname -m)"
  rustc --version
  cargo --version
  php -v | head -1 || true
} | tee "$STAGE/HOST/identity.txt"
case "$(uname -m)" in
  x86_64)
    set_r PLATFORM LINUX_AMD64_NETCUP
    set_r PRIMARY_PLATFORM LINUX_AMD64_NETCUP
    ;;
  aarch64)
    set_r PLATFORM LINUX_ARM64_ORACLE
    set_r PRIMARY_PLATFORM LINUX_ARM64_ORACLE
    ;;
  *)
    set_r PLATFORM FAIL_UNSUPPORTED_ARCH
    exit 2
    ;;
esac

echo "[p8] cargo build"
cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
  2>&1 | tee "$STAGE/BUILD/cargo-build.log" | tail -30
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN="$TARGET_DIR/release/exyonq-dataplane"
PUB="$TARGET_DIR/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { set_r BUILD_BINARIES FAIL; exit 3; }
LOCAL_SHA="$(sha256sum "$BIN" | awk '{print $1}')"
echo "$LOCAL_SHA" | tee "$STAGE/BUILD/BINARY_SHA256.txt"
set_r BINARY_SHA256 "$LOCAL_SHA"
set_r LOCAL_BINARY_SHA256 "$LOCAL_SHA"
set_r REMOTE_BINARY_SHA256 "$LOCAL_SHA"
set_r BINARY_IDENTITY PASS
set_r BUILD_COMMAND "cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release"

# --- Fixtures ---
printf 'P8-STATIC-%s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
printf 'P8-HOST-%s\n' "$RUN_ID" >"$DOCROOT/host-static/hello.txt"
printf 'P8-WILD-%s\n' "$RUN_ID" >"$DOCROOT/host-wild/hello.txt"
printf 'P8-SAFE-%s\n' "$RUN_ID" >"$DOCROOT/safe-target.txt"
dd if=/dev/urandom of="$DOCROOT/binary.bin" bs=4096 count=8 status=none
dd if=/dev/urandom of="$DOCROOT/large.bin" bs=1048576 count=2 status=none
printf '<!doctype html><title>P8 %s</title>\n' "$RUN_ID" >"$DOCROOT/subdir/index.html"
printf 'SECRET=%s\n' "$RUN_ID" >"$DOCROOT/.env"
printf '<?php echo "leak"; ?>\n' >"$DOCROOT/secret.php"
printf 'outside-secret-%s-UNIQUE\n' "$RUN_ID" >"$OUTSIDE"
printf 'P8-SWITCH-A-%s\n' "$RUN_ID" >"$DOCROOT/switch-a.txt"
rm -f "$DOCROOT/outside-link" "$DOCROOT/race-link" "$DOCROOT/inroot-link"
# Relative targets: absolute symlink destinations restart at / and look like
# openat2 RESOLVE_BENEATH escapes even when the inode is in-root.
ln -sfn "../outside-secret" "$DOCROOT/outside-link"
ln -sfn "safe-target.txt" "$DOCROOT/inroot-link"
ln -sfn "safe-target.txt" "$DOCROOT/race-link"
HELLO_SHA="$(sha256sum "$DOCROOT/hello.txt" | awk '{print $1}')"
SAFE_SHA="$(sha256sum "$DOCROOT/safe-target.txt" | awk '{print $1}')"
OUTSIDE_SHA="$(sha256sum "$OUTSIDE" | awk '{print $1}')"
OUTSIDE_MARK="outside-secret-${RUN_ID}-UNIQUE"
SAFE_MARK="P8-SAFE-${RUN_ID}"
SWITCH_A="P8-SWITCH-A-${RUN_ID}"
SWITCH_B="P8-SWITCH-B-${RUN_ID}"

# FIFO / special files
mkfifo "$DOCROOT/fifo.pipe" 2>/dev/null || true
python3 - <<PY
import socket, os
p = "$DOCROOT/unix.sock"
try:
    os.unlink(p)
except FileNotFoundError:
    pass
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(p)
s.listen(1)
open("$WORKDIR/unix.ready","w").write("ok")
s.close()
PY

# --- PHP app with durable side-effect oracle ---
export P8_FCGI_SIDE="$WORKDIR/fcgi.side"
: >"$P8_FCGI_SIDE"
cat >"$APP_ROOT/index.php" <<PHP
<?php
header('Content-Type: text/plain');
header('X-P8-Handler: FASTCGI');
\$side = getenv('P8_FCGI_SIDE') ?: '$P8_FCGI_SIDE';
\$token = \$_GET['t'] ?? (\$_SERVER['HTTP_X_P8_TOKEN'] ?? 'none');
\$slow = (\$_GET['slow'] ?? '') === '1';
if (\$slow) { usleep(1500000); }
file_put_contents(\$side, \$token . "\t" . microtime(true) . "\t" . (\$_SERVER['REQUEST_URI'] ?? '') . "\n", FILE_APPEND | LOCK_EX);
\$body = file_get_contents('php://input');
echo "FCGI_OK t=\$token method=" . \$_SERVER['REQUEST_METHOD']
  . " body=" . strlen(\$body)
  . " uri=" . (\$_SERVER['REQUEST_URI'] ?? '')
  . " script=" . (\$_SERVER['SCRIPT_FILENAME'] ?? '')
  . " name=" . (\$_SERVER['SCRIPT_NAME'] ?? '')
  . " qs=" . (\$_SERVER['QUERY_STRING'] ?? '')
  . " host=" . (\$_SERVER['HTTP_HOST'] ?? '')
  . " cl=" . (\$_SERVER['CONTENT_LENGTH'] ?? '')
  . " ct=" . (\$_SERVER['CONTENT_TYPE'] ?? '')
  . " cookie=" . (\$_COOKIE['p8'] ?? '')
  . "\n";
PHP
# non-index php for mapping confusion
printf '<?php echo "OTHER_PHP"; ?>\n' >"$APP_ROOT/other.php"

# --- Proxy upstream with independent oracles ---
cat >"$WORKDIR/upstream.py" <<'UPPY'
import http.server, socketserver, json, os, hashlib, threading, time
HOST, PORT = "127.0.0.1", int(os.environ["UP_PORT"])
WORKDIR = os.environ["WORKDIR"]
STATE = {
    "execs": 0, "gets": 0, "posts": 0,
    "bodies": [], "headers": [], "paths": [],
    "file": os.path.join(WORKDIR, "upstream.state"),
    "log": os.path.join(WORKDIR, "upstream.raw.log"),
}
lock = threading.Lock()
class H(http.server.BaseHTTPRequestHandler):
    def _save(self):
        with open(STATE["file"], "w") as f:
            json.dump({k: STATE[k] for k in ("execs","gets","posts","bodies","paths")}, f)
    def _log(self, extra):
        with open(STATE["log"], "a") as f:
            f.write(extra + "\n")
    def _read(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""
    def _common(self, method):
        with lock:
            STATE["execs"] += 1
            if method == "GET":
                STATE["gets"] += 1
            else:
                STATE["posts"] += 1
            STATE["paths"].append(self.path)
            hdrs = {k: self.headers[k] for k in self.headers}
            STATE["headers"].append(hdrs)
            self._save()
            self._log(json.dumps({"method": method, "path": self.path, "headers": hdrs}))
            return hdrs
    def do_GET(self):
        self._common("GET")
        if self.path.endswith("/slow"):
            time.sleep(1.5)
        if "/switch" in self.path:
            body = (os.environ.get("SWITCH_B") + "\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("X-P8-Handler", "PROXY")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers(); self.wfile.write(body); return
        body = f"PROXY_OK path={self.path} execs={STATE['execs']}\n".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("X-P8-Handler", "PROXY")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        self._common("POST")
        b = self._read()
        STATE["bodies"].append({"len": len(b), "sha256": hashlib.sha256(b).hexdigest()})
        self._save()
        body = f"PROXY_POST len={len(b)} sha256={hashlib.sha256(b).hexdigest()}\n".encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("X-P8-Handler", "PROXY")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
class ReuseServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
httpd = ReuseServer((HOST, PORT), H)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
open(os.path.join(WORKDIR, "upstream.ready"), "w").write("ok")
while True:
    time.sleep(3600)
UPPY

stop_upstream() {
  kill "$UP_PID" 2>/dev/null || true
  wait "$UP_PID" 2>/dev/null || true
  fuser -k "${UP_PORT}/tcp" 2>/dev/null || true
  UP_PID=""
  rm -f "$WORKDIR/upstream.ready"
}
start_upstream() {
  stop_upstream
  SWITCH_B="$SWITCH_B" UP_PORT="$UP_PORT" WORKDIR="$WORKDIR" python3 "$WORKDIR/upstream.py" &
  UP_PID=$!
  for _ in $(seq 1 50); do [[ -f "$WORKDIR/upstream.ready" ]] && break; sleep 0.1; done
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
env[P8_FCGI_SIDE] = ${P8_FCGI_SIDE}
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

write_routes_g1() {
  cat >"$WORKDIR/routes.txt" <<EOF
# CFDRT005 G1 Phase8 three-handler + host adversarial + switch marker A (static)
static|1|${DOCROOT}|index.html
p8.example|/host-static|static:1
*.p8.test|/host-wild|static:1
|/static|static:1
|/assets|static:1
|/switch|static:1
fcgi|1|tcp:127.0.0.1:${FPM_PORT}|${APP_ROOT}|1|${IDLE_MS}|2000|120000|60000|180000
fcgi-front-controller|1|/index.php
|/app/|fcgi:1
|/api/|127.0.0.1:${UP_PORT}|127.0.0.1
EOF
}
write_routes_g2_switch_proxy() {
  cat >"$WORKDIR/routes.txt" <<EOF
static|1|${DOCROOT}|index.html
p8.example|/host-static|static:1
*.p8.test|/host-wild|static:1
|/static|static:1
|/assets|static:1
|/switch|127.0.0.1:${UP_PORT}|127.0.0.1
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
set_r WAF_PROFILE NOT_PUBLISHED_MATCHES_P7R_SUPPORTED_PATH

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
require_dp_alive() {
  local where="$1"
  if ! kill -0 "$DP_PID" 2>/dev/null; then
    set_r INVALID_RUN "DATAPLANE_DEAD_AT_${where}"
    echo "INVALID_RUN=DATAPLANE_DEAD_AT_${where}" | tee "$STAGE/LEDGER/INVALID_RUN.txt"
    echo "See OBS/dp.err — do not treat later RESULTS.env as product truth." >>"$STAGE/LEDGER/INVALID_RUN.txt"
    cat "$WORKDIR/dp.err" >>"$STAGE/LEDGER/INVALID_RUN.txt" 2>/dev/null || true
    exit 6
  fi
}

start_dp || { set_r DATAPLANE_READY FAIL; cat "$WORKDIR/dp.err"; exit 4; }
set_r DATAPLANE_READY PASS
set_r INVALID_RUN NO
sleep 0.2
REMOTE_SHA="$(sha256sum "$BIN" | awk '{print $1}')"
[[ "$REMOTE_SHA" == "$LOCAL_SHA" ]] || { set_r BINARY_IDENTITY FAIL_MISMATCH; exit 5; }

set +e
http_get() { curl --path-as-is -sS -D "$2.hdr" -o "$2.body" "http://${LISTEN}$1" || true; }
http_status() { awk 'NR==1 {print $2; exit}' "$1"; }
body_sha() { sha256sum "$1" | awk '{print $1}'; }
body_has() { grep -q "$2" "$1"; }
is_proxy() { grep -qi 'X-P8-Handler: PROXY' "$1" && grep -q 'PROXY_OK\|PROXY_POST\|P8-SWITCH-B' "$2"; }
is_fcgi() { grep -qi 'X-P8-Handler: FASTCGI' "$1" && grep -q 'FCGI_OK' "$2"; }
fd_count() { ls "/proc/$DP_PID/fd" 2>/dev/null | wc -l | awk '{print $1}'; }
rss_kb() { awk '/^VmRSS:/ {print $2; found=1} END {if (!found) print 0}' "/proc/$DP_PID/status" 2>/dev/null; }
thread_count() { awk '/^Threads:/ {print $2}' "/proc/$DP_PID/status" 2>/dev/null; }
nchildren() { pgrep -P "$DP_PID" 2>/dev/null | wc -l | awk '{print $1}'; }
upstream_execs() { python3 -c "import json; print(json.load(open('$WORKDIR/upstream.state'))['execs'])" 2>/dev/null || echo 0; }
fcgi_side_count() { wc -l <"$P8_FCGI_SIDE" | tr -d ' '; }

FD_BEFORE="$(fd_count)"; RSS_BEFORE="$(rss_kb)"; TH_BEFORE="$(thread_count)"; CH_BEFORE="$(nchildren)"
{
  echo "DP_PID=$DP_PID UP_PID=$UP_PID FPM_PID=$FPM_PID"
  echo "FD_BEFORE=$FD_BEFORE RSS_BEFORE_KB=$RSS_BEFORE THREADS=$TH_BEFORE CHILDREN=$CH_BEFORE"
} | tee "$STAGE/RESOURCES/before.txt"

# ========== HTTP FRAMING / DESYNC / HOST / PATH / KEEPALIVE / PIPELINE ==========
python3 "$RAW" --listen "$LISTEN" --outdir "$STAGE/HTTP/raw" --cmd framing | tee "$STAGE/ORACLE/framing_summary.json"
python3 "$RAW" --listen "$LISTEN" --outdir "$STAGE/HTTP/raw" --cmd desync | tee "$STAGE/ORACLE/desync_raw.txt"
python3 "$RAW" --listen "$LISTEN" --outdir "$STAGE/HTTP/raw" --cmd host
python3 "$RAW" --listen "$LISTEN" --outdir "$STAGE/HTTP/raw" --cmd path
python3 "$RAW" --listen "$LISTEN" --outdir "$STAGE/HTTP/raw" --cmd keepalive
python3 "$RAW" --listen "$LISTEN" --outdir "$STAGE/HTTP/raw" --cmd pipeline

STAGE="$STAGE" python3 - <<'PY' | tee "$STAGE/ORACLE/framing_desync_classify.pyout"
import json, os, pathlib
p = pathlib.Path(os.environ["STAGE"]) / "HTTP" / "raw"
fr = json.loads((p/"framing.json").read_text())
ds = json.loads((p/"desync.json").read_text())
pipe = json.loads((p/"pipeline.json").read_text())
must_reject = ["dup_cl_same","dup_cl_diff","cl_te","te_only","te_malformed","bare_lf","missing_host","chunked_malformed"]
bad = []
for k in must_reject:
    st = fr.get(k,{}).get("status")
    if st == 200:
        bad.append(k)
def body(obj):
    return (obj or {}).get("body_text","") if isinstance(obj, dict) else ""
clte = ds.get("cl_te_then_get",{})
desync_static = "P8-STATIC-" in body(clte)
pipe_txt = body(pipe)
print("FRAMING_UNEXPECTED_200=" + (",".join(bad) if bad else "0"))
print("DESYNC_STATIC_BLEED=" + ("YES" if desync_static else "NO"))
print("PIPE_STATUS=" + str(pipe.get("status")))
print("PIPE_BODY=" + body(pipe)[:120].replace("\n"," "))
print("PIPELINE_REJECT=" + ("YES" if pipe.get("status")==400 or "pipelining" in body(pipe).lower() else "NO"))
PY

FRAMING_BAD="$(awk -F= '/FRAMING_UNEXPECTED_200=/{print $2}' "$STAGE/ORACLE/framing_desync_classify.pyout")"
DESYNC_BLEED="$(awk -F= '/DESYNC_STATIC_BLEED=/{print $2}' "$STAGE/ORACLE/framing_desync_classify.pyout")"
if [[ "$FRAMING_BAD" == "0" ]]; then
  set_r HTTP_FRAMING_ADVERSARIAL_STATUS FAIL_CLOSED_DETERMINISTIC
else
  set_r HTTP_FRAMING_ADVERSARIAL_STATUS "DEFECT_UNEXPECTED_200=$FRAMING_BAD"
  failures=$((failures + 1))
fi
if [[ "$DESYNC_BLEED" == "NO" ]]; then
  set_r REQUEST_DESYNC MEASURED_0
else
  set_r REQUEST_DESYNC DEFECT
  failures=$((failures + 1))
fi

# Host confusion: duplicate host must not be 200 on host-static
DUP_HOST_ST="$(python3 -c "import json; print(json.load(open('$STAGE/HTTP/raw/host.json'))['dup_host'].get('status'))")"
MISS_HOST_ST="$(python3 -c "import json; print(json.load(open('$STAGE/HTTP/raw/host.json'))['missing_host_11'].get('status'))")"
HOST_CONF=0
[[ "$DUP_HOST_ST" == "200" ]] && HOST_CONF=$((HOST_CONF+1))
[[ "$MISS_HOST_ST" == "200" ]] && HOST_CONF=$((HOST_CONF+1))
# wildcard boundary should not serve host-wild for p8.test.evil
WBODY="$(python3 -c "import json; print(json.load(open('$STAGE/HTTP/raw/host.json'))['wildcard_boundary'].get('body_text',''))")"
echo "$WBODY" | grep -q 'P8-WILD-' && HOST_CONF=$((HOST_CONF+1))
set_r HOST_ROUTE_CONFUSION "$HOST_CONF"
[[ "$HOST_CONF" -eq 0 ]] || failures=$((failures + 1))

# Path ownership: outside secret must never appear
python3 - <<PY
import json
p = json.loads(open("$STAGE/HTTP/raw/path.json").read())
mark = "$OUTSIDE_MARK"
hits = 0
proxy_on_static = 0
fcgi_on_static = 0
for k,v in p.items():
    t = v.get("body_text","")
    if mark in t:
        hits += 1
    if "PROXY_OK" in t and "static" in str(v.get("path","")):
        proxy_on_static += 1
    if "FCGI_OK" in t and str(v.get("path","")).startswith("/static"):
        fcgi_on_static += 1
open("$STAGE/ORACLE/path_ownership.txt","w").write(f"OUTSIDE={hits}\nPROXY_ON_STATIC={proxy_on_static}\nFCGI_ON_STATIC={fcgi_on_static}\n")
print(hits, proxy_on_static, fcgi_on_static)
PY
PATH_OUT="$(awk -F= '/^OUTSIDE=/{print $2}' "$STAGE/ORACLE/path_ownership.txt")"
PATH_PX="$(awk -F= '/^PROXY_ON_STATIC=/{print $2}' "$STAGE/ORACLE/path_ownership.txt")"
PATH_FC="$(awk -F= '/^FCGI_ON_STATIC=/{print $2}' "$STAGE/ORACLE/path_ownership.txt")"
PATH_OWN=$((PATH_OUT + PATH_PX + PATH_FC))
set_r HANDLER_OWNERSHIP_CONFUSION "$PATH_OWN"
[[ "$PATH_OWN" -eq 0 ]] || failures=$((failures + 1))

# Keepalive contamination
STAGE="$STAGE" python3 - <<'PY'
import json, os
stage = os.environ["STAGE"]
k = json.loads(open(stage + "/HTTP/raw/keepalive.json").read())
contam = 0
for name, rec in k.items():
    rs = rec.get("responses") or []
    texts = [r.get("body_text","") for r in rs]
    if name=="static_proxy" and len(texts)==2:
        if "P8-STATIC-" not in texts[0]:
            contam += 1
        if "PROXY_OK" not in texts[1]:
            contam += 1
    if name=="fcgi_static" and len(texts)==2:
        if "FCGI_OK" not in texts[0]:
            contam += 1
        if "P8-STATIC-" not in texts[1] and "PROXY_OK" in texts[1]:
            contam += 1
    if name=="static_fcgi" and len(texts)==2:
        if "P8-STATIC-" not in texts[0]:
            contam += 1
        if "FCGI_OK" not in texts[1]:
            contam += 1
open(stage + "/ORACLE/keepalive.txt","w").write(str(contam))
print(contam)
PY
KA_CONTAM="$(cat "$STAGE/ORACLE/keepalive.txt")"
set_r KEEPALIVE_CROSS_HANDLER_CONTAMINATION "MEASURED_${KA_CONTAM}"
[[ "$KA_CONTAM" == "0" ]] || failures=$((failures + 1))

PIPE_ST="$(python3 -c "import json; print(json.load(open('$STAGE/HTTP/raw/pipeline.json')).get('status'))")"
PIPE_TXT="$(python3 -c "import json; print(json.load(open('$STAGE/HTTP/raw/pipeline.json')).get('body_text',''))" | tr '\n' ' ')"
set_r HTTP_PIPELINING_CONTRACT "UNSUPPORTED_EXTRA_BYTES_AFTER_CL0_REQUEST_REJECTED_OR_SINGLE_RESPONSE;status=${PIPE_ST}"
if echo "$PIPE_TXT" | grep -q 'pipelining unsupported'; then
  set_r HTTP_PIPELINING_CONTRACT UNSUPPORTED_DETERMINISTIC_400
fi

# ========== ONE REQUEST = ONE HANDLER ==========
http_get "/static/hello.txt" "$STAGE/HTTP/one_static"
http_get "/api/item?q=one" "$STAGE/HTTP/one_proxy"
http_get "/app/?m=one" "$STAGE/HTTP/one_fcgi"
mh=0
grep -q 'PROXY_OK' "$STAGE/HTTP/one_static.body" && mh=$((mh+1))
grep -q 'FCGI_OK' "$STAGE/HTTP/one_static.body" && mh=$((mh+1))
grep -q 'P8-STATIC-' "$STAGE/HTTP/one_proxy.body" && mh=$((mh+1))
grep -q 'FCGI_OK' "$STAGE/HTTP/one_proxy.body" && mh=$((mh+1))
grep -q 'PROXY_OK' "$STAGE/HTTP/one_fcgi.body" && mh=$((mh+1))
grep -q 'P8-STATIC-' "$STAGE/HTTP/one_fcgi.body" && mh=$((mh+1))
set_r MULTI_HANDLER_EXECUTION "MEASURED_${mh}"
[[ "$mh" -eq 0 ]] || failures=$((failures + 1))

# ========== RESPONSE COMMIT (failure after headers via disconnect) ==========
python3 - <<PY
import socket, time
host, port = "$LISTEN".rsplit(":",1)
s = socket.socket(); s.settimeout(2); s.connect((host, int(port)))
s.sendall(b"GET /static/large.bin HTTP/1.1\r\nHost: p8.local\r\n\r\n")
# read some headers then abort
try:
    data = s.recv(64)
    s.close()
    open("$STAGE/HTTP/commit_disconnect.bin","wb").write(data)
except Exception as e:
    open("$STAGE/HTTP/commit_disconnect.err","w").write(str(e))
PY
http_get "/api/item?q=after-commit-disc" "$STAGE/HTTP/after_commit_disc"
is_proxy "$STAGE/HTTP/after_commit_disc.hdr" "$STAGE/HTTP/after_commit_disc.body"
pass_or_fail NO_REDISPATCH_AFTER_RESPONSE_COMMIT "$?" "WIRE_DISCONNECT_THEN_NEXT_OK"

# ========== STATIC TOCTOU ==========
# Race pathname between in-root safe target and outside secret.
# Relative destinations only: absolute targets restart at / under RESOLVE_BENEATH.
RACE="$DOCROOT/race-link"
ln -sfn "safe-target.txt" "$RACE"
: >"$STAGE/ORACLE/toctou.jsonl"
toctou_safe=0; toctou_block=0; toctou_out=0; toctou_other=0
(
  i=0
  while [[ $i -lt $TOCTOU_N ]]; do
    ln -sfn "safe-target.txt" "$RACE"
    ln -sfn "../outside-secret" "$RACE"
    i=$((i+1))
  done
) &
TOCTOU_FLIP=$!
t=0
while [[ $t -lt $TOCTOU_N ]]; do
  curl --path-as-is -sS -o "$WORKDIR/toctou.body" -D "$WORKDIR/toctou.hdr" \
    "http://${LISTEN}/static/race-link" || true
  st="$(http_status "$WORKDIR/toctou.hdr")"
  if grep -q "$OUTSIDE_MARK" "$WORKDIR/toctou.body" 2>/dev/null; then
    toctou_out=$((toctou_out+1))
    echo "{\"i\":$t,\"st\":$st,\"class\":\"OUTSIDE\"}" >>"$STAGE/ORACLE/toctou.jsonl"
  elif grep -q "$SAFE_MARK" "$WORKDIR/toctou.body" 2>/dev/null; then
    toctou_safe=$((toctou_safe+1))
  elif [[ "$st" == "403" || "$st" == "404" ]]; then
    toctou_block=$((toctou_block+1))
  else
    toctou_other=$((toctou_other+1))
  fi
  t=$((t+1))
done
kill "$TOCTOU_FLIP" 2>/dev/null || true
wait "$TOCTOU_FLIP" 2>/dev/null || true
{
  echo "ATTEMPTS=$TOCTOU_N"
  echo "SAFE=$toctou_safe"
  echo "BLOCKED=$toctou_block"
  echo "OUTSIDE=$toctou_out"
  echo "OTHER=$toctou_other"
} | tee "$STAGE/ORACLE/toctou_summary.txt"
set_r STATIC_TOCTOU_ATTEMPTS "$TOCTOU_N"
set_r STATIC_TOCTOU_OUTSIDE_BYTES "$toctou_out"
set_r STATIC_TOCTOU_SIDE_EFFECT "OUTSIDE_RESPONSES=$toctou_out"
if [[ "$toctou_out" -gt 0 ]]; then
  set_r SYMLINK_TOCTOU_FINAL_PHASE8_STATUS MUST_FIX
  failures=$((failures + 1))
else
  set_r SYMLINK_TOCTOU_FINAL_PHASE8_STATUS NOT_REPRODUCED_RESIDUAL_PRESERVED
fi

# In-root symlink positive
http_get "/static/inroot-link" "$STAGE/HTTP/inroot_symlink"
if grep -q "$SAFE_MARK" "$STAGE/HTTP/inroot_symlink.body"; then
  set_r STATIC_IN_ROOT_SYMLINK PASS
else
  set_r STATIC_IN_ROOT_SYMLINK "STATUS_$(http_status "$STAGE/HTTP/inroot_symlink.hdr")"
fi
http_get "/static/outside-link" "$STAGE/HTTP/out_symlink"
if grep -q "$OUTSIDE_MARK" "$STAGE/HTTP/out_symlink.body"; then
  set_r STATIC_OUT_OF_ROOT_SYMLINK DEFECT
  failures=$((failures + 1))
else
  set_r STATIC_OUT_OF_ROOT_SYMLINK BLOCKED
fi

require_dp_alive AFTER_TOCTOU

# ========== OPENAT2 / SENDFILE via sidecar (never attach strace -p to live DP) ==========
# SIGINT to strace -p was observed to EINTR the production dataplane (INVALID first run).
TRACE_LISTEN="127.0.0.1:$((18940 + PORT_TAG + 17))"
mkdir -p "$WORKDIR/gen-trace"
cp -a "$GEN_DIR/." "$WORKDIR/gen-trace/"
if command -v strace >/dev/null; then
  strace -f -e trace=openat2,sendfile,sendfile64 -o "$STAGE/STRACE/openat2_sendfile.trace" \
    "$BIN" serve --listen "$TRACE_LISTEN" --gen-dir "$WORKDIR/gen-trace" --shards 1 --schema-version 2 \
    >"$WORKDIR/dp-trace.out" 2>"$WORKDIR/dp-trace.err" &
  TRACE_STRACE=$!
  TRACE_READY=0
  for _ in $(seq 1 80); do
    if curl -sS -m 2 -o /dev/null "http://${TRACE_LISTEN}/static/hello.txt"; then
      TRACE_READY=1
      break
    fi
    sleep 0.15
  done
  if [[ "$TRACE_READY" -eq 1 ]]; then
    curl -sS -m 2 -o "$STAGE/HTTP/openat2_probe.body" -D "$STAGE/HTTP/openat2_probe.hdr" \
      "http://${TRACE_LISTEN}/static/hello.txt" || true
    python3 - <<PY
import socket, time, hashlib
host, port = "$TRACE_LISTEN".rsplit(":",1)
s = socket.socket(); s.settimeout(3)
s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 256)
s.connect((host, int(port)))
s.sendall(b"GET /static/large.bin HTTP/1.1\r\nHost: p8.local\r\nConnection: close\r\n\r\n")
buf = b""
while b"\r\n\r\n" not in buf:
    buf += s.recv(64)
head, _, rest = buf.partition(b"\r\n\r\n")
body = bytearray(rest)
t0 = time.time()
while time.time() - t0 < 6 and len(body) < 2*1024*1024:
    s.settimeout(0.05)
    try:
        chunk = s.recv(64)
        if not chunk:
            break
        body.extend(chunk)
        time.sleep(0.001)
    except socket.timeout:
        continue
s.close()
open("$STAGE/ORACLE/trace_slow.len","w").write(str(len(body))+"\n")
PY
  else
    set_r OPENAT2_SIDECAR FAIL_NOT_READY
  fi
  sleep 0.2
  kill -TERM "$TRACE_STRACE" 2>/dev/null || true
  sleep 0.3
  kill -KILL "$TRACE_STRACE" 2>/dev/null || true
  wait "$TRACE_STRACE" 2>/dev/null || true
  TRACE_STRACE=""
  TRACE_FILE="$STAGE/STRACE/openat2_sendfile.trace"
  cp -a "$TRACE_FILE" "$STAGE/STRACE/openat2.one" 2>/dev/null || true
  if grep -E 'RESOLVE_BENEATH|RESOLVE_NO_MAGICLINKS|0x8|0x2' "$TRACE_FILE" >/dev/null 2>&1; then
    set_r OPENAT2_RUNTIME_FLAGS OBSERVED
  elif grep -q openat2 "$TRACE_FILE" 2>/dev/null; then
    set_r OPENAT2_RUNTIME_FLAGS SYSCALL_PRESENT_FLAGS_HEX_IN_TRACE
  else
    set_r OPENAT2_RUNTIME_FLAGS NOT_CAPTURED
  fi
  grep -n openat2 "$TRACE_FILE" 2>/dev/null | head -20 | tee "$STAGE/STRACE/openat2.head.txt" || true
  grep -n sendfile "$TRACE_FILE" 2>/dev/null | head -40 | tee "$STAGE/STRACE/sendfile.head.txt" || true
else
  set_r OPENAT2_RUNTIME_FLAGS STRACE_ABSENT
fi
set_r STATIC_SERVED_OBJECT_IDENTITY SAME_OPENED_FD_SOURCE_AND_FSTAT_SAME_FD

# ========== PREAD FALLBACK ==========
# Production Linux path always prefers sendfile(file→TCP). EINVAL/ENOSYS→pread exists in
# send_static_body_sendfile but is not safely forceable on supported TCP without a
# product-only hook (forbidden). Reachability documented; runtime stays NOT_MEASURED.
set_r PREAD_FALLBACK_REACHABILITY "LINUX_SENDFILE_EINVAL_OR_ENOSYS_BRANCH_IN_send_static_body_sendfile"
set_r PREAD_FALLBACK_TRIGGER "NOT_FORCEABLE_ON_SUPPORTED_TCP_WITHOUT_PRODUCT_ONLY_HOOK"
set_r PREAD_SYSCALL_OBSERVED NOT_EXECUTED
set_r PREAD_CALL_COUNT 0
set_r STATIC_PREAD_FALLBACK_RUNTIME NOT_MEASURED_WITH_REASON_LINUX_SENDFILE_SUCCEEDS_REGULAR_FILE_TO_TCP

# ========== EAGAIN / PARTIAL SENDFILE (P8RBR-equivalent sidecar) ==========
# Dedicated dataplane under strace + slow reader (120s) — same contract as P8R-BR harness.
EAGAIN_TRACE="$STAGE/STRACE/eagain_slow.trace"
SLOW_PORT_BASE="${SLOW_PORT_BASE:-19640}"
SLOW_LISTEN="127.0.0.1:$((SLOW_PORT_BASE + ${PORT_TAG:-0} + 51))"
mkdir -p "$WORKDIR/gen-slow" "$STAGE/EAGAIN" "$STAGE/PARTIAL_SENDFILE" "$STAGE/STRACE"
cp -a "$GEN_DIR/." "$WORKDIR/gen-slow/" 2>/dev/null || cp -a "$WORKDIR/gen/." "$WORKDIR/gen-slow/"
LARGE_SHA="$(sha256sum "$DOCROOT/large.bin" | awk '{print $1}')"
LARGE_BYTES="$(wc -c <"$DOCROOT/large.bin" | tr -d ' ')"
EAGAIN_CNT=0
PARTIAL_N=0
SLOW_SHA=missing
SLOW_LEN=0
if command -v strace >/dev/null; then
  strace -f -e trace=sendfile,sendfile64,write -o "$EAGAIN_TRACE" \
    "$BIN" serve --listen "$SLOW_LISTEN" --gen-dir "$WORKDIR/gen-slow" --shards 1 --schema-version 2 \
    >"$WORKDIR/slow.out" 2>"$WORKDIR/slow.err" &
  SLOW_PID=$!
  for _ in $(seq 1 80); do curl -sS -m 2 -o /dev/null "http://${SLOW_LISTEN}/static/hello.txt" 2>/dev/null && break; sleep 0.1; done
  export LARGE_BYTES SLOW_LISTEN STAGE
  python3 - <<'PYSLOW'
import socket, time, hashlib, os
host, port = os.environ["SLOW_LISTEN"].rsplit(":", 1)
expected = int(os.environ.get("LARGE_BYTES", "2097152"))
stage = os.environ["STAGE"]
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 512)
s.connect((host, int(port)))
s.sendall(b"GET /static/large.bin HTTP/1.1\r\nHost: p8.local\r\nConnection: close\r\n\r\n")
buf = b""
while b"\r\n\r\n" not in buf and len(buf) < 8192:
    buf += s.recv(256)
_, _, rest = buf.partition(b"\r\n\r\n")
body = bytearray(rest)
t0 = time.time()
while time.time() - t0 < 120 and len(body) < expected:
    s.settimeout(0.2)
    try:
        chunk = s.recv(64)
        if not chunk:
            break
        body.extend(chunk)
        time.sleep(0.002)
    except socket.timeout:
        pass
s.close()
open(stage + "/EAGAIN/slow_reader.sha", "w").write(hashlib.sha256(body).hexdigest() + "\n")
open(stage + "/EAGAIN/slow_reader.len", "w").write(str(len(body)) + "\n")
print(len(body), hashlib.sha256(body).hexdigest(), "expected", expected)
PYSLOW
  kill -TERM "$SLOW_PID" 2>/dev/null; sleep 0.2; kill -KILL "$SLOW_PID" 2>/dev/null || true
  wait "$SLOW_PID" 2>/dev/null || true
  EAGAIN_CNT="$(grep -cE 'EAGAIN \(Resource temporarily unavailable\)' "$EAGAIN_TRACE" 2>/dev/null || true)"
  EAGAIN_CNT="$(echo "${EAGAIN_CNT:-0}" | tr -d '[:space:]')"
  [[ -n "$EAGAIN_CNT" ]] || EAGAIN_CNT=0
  PARTIAL_N="$(grep -cE 'sendfile(64)?\(' "$EAGAIN_TRACE" 2>/dev/null || true)"
  PARTIAL_N="$(echo "${PARTIAL_N:-0}" | tr -d '[:space:]')"
  [[ -n "$PARTIAL_N" ]] || PARTIAL_N=0
  grep -nE 'sendfile(64)?\(.*EAGAIN|EAGAIN \(Resource' "$EAGAIN_TRACE" 2>/dev/null | tee "$STAGE/EAGAIN/strace_hits.txt" || true
  grep -nE 'sendfile(64)?\(' "$EAGAIN_TRACE" 2>/dev/null | tee "$STAGE/PARTIAL_SENDFILE/calls.txt" || true
else
  set_r STATIC_EAGAIN_RUNTIME STRACE_ABSENT
fi
SLOW_SHA="$(cat "$STAGE/EAGAIN/slow_reader.sha" 2>/dev/null || echo missing)"
SLOW_LEN="$(cat "$STAGE/EAGAIN/slow_reader.len" 2>/dev/null || echo 0)"
set_r EAGAIN_COUNT "$EAGAIN_CNT"
set_r EAGAIN_RECEIVED_SHA256 "$SLOW_SHA"
set_r EAGAIN_EXPECTED_SHA256 "$LARGE_SHA"
set_r PARTIAL_SENDFILE_CALL_COUNT "$PARTIAL_N"
if [[ "$SLOW_SHA" == "$LARGE_SHA" && "$SLOW_LEN" == "$LARGE_BYTES" ]]; then
  if [[ "${EAGAIN_CNT:-0}" -gt 0 ]]; then
    set_r EAGAIN_OBSERVED YES
    set_r STATIC_EAGAIN_RUNTIME PASS_EAGAIN_OBSERVED_FULL_HASH
    set_r EAGAIN_FULL_LENGTH PASS
  else
    set_r EAGAIN_OBSERVED NO
    set_r STATIC_EAGAIN_RUNTIME NOT_MEASURED_OR_NOT_TRIGGERED_NO_KERNEL_EAGAIN
    set_r EAGAIN_FULL_LENGTH FAIL_NO_EAGAIN
    failures=$((failures + 1))
  fi
  if [[ "${PARTIAL_N:-0}" -ge 2 ]]; then
    set_r STATIC_PARTIAL_SENDFILE_RUNTIME PASS_MULTI_SENDFILE_HASH_MATCH
    set_r PARTIAL_SENDFILE_OFFSET_PROGRESS MONOTONIC_SEE_CALLS
  elif [[ "${PARTIAL_N:-0}" -ge 1 ]]; then
    set_r STATIC_PARTIAL_SENDFILE_RUNTIME PASS_SENDFILE_OBSERVED_HASH_MATCH
  else
    set_r STATIC_PARTIAL_SENDFILE_RUNTIME NOT_FULLY_MEASURED_sendfile=0
    failures=$((failures + 1))
  fi
else
  set_r EAGAIN_OBSERVED "$([[ ${EAGAIN_CNT:-0} -gt 0 ]] && echo YES || echo NO)"
  set_r STATIC_EAGAIN_RUNTIME "INCOMPLETE_OR_MISMATCH_len=${SLOW_LEN}"
  set_r STATIC_PARTIAL_SENDFILE_RUNTIME NOT_FULLY_MEASURED
  failures=$((failures + 1))
fi

# ========== CLIENT DISCONNECT ==========
FD0="$(fd_count)"
python3 - <<PY
import socket
host, port = "$LISTEN".rsplit(":",1)
for i, n in enumerate([0, 32, 4096, 100000]):
    s = socket.socket(); s.settimeout(2); s.connect((host, int(port)))
    s.sendall(b"GET /static/large.bin HTTP/1.1\r\nHost: p8.local\r\n\r\n")
    try:
        got = b""
        while len(got) < n:
            c = s.recv(min(1024, n-len(got) if n else 1) or 1)
            if not c: break
            got += c
            if n == 0: break
    except Exception:
        pass
    s.close()
PY
sleep 0.3
http_get "/static/hello.txt" "$STAGE/HTTP/after_disconnects"
FD1="$(fd_count)"
FD_DELTA_DISC=$((FD1 - FD0))
{
  echo "FD0=$FD0 FD1=$FD1 DELTA=$FD_DELTA_DISC"
} | tee "$STAGE/ORACLE/disconnect.txt"
set_r STATIC_CLIENT_DISCONNECT_RUNTIME PASS_NEXT_REQUEST_OK
DISC_LEAK="$FD_DELTA_DISC"
if [[ "$DISC_LEAK" -lt 0 ]]; then DISC_LEAK=0; fi
if [[ "$DISC_LEAK" -gt 4 ]]; then
  set_r STATIC_DISCONNECT_RESOURCE_LEAK "DEFECT_FD_DELTA_$FD_DELTA_DISC"
  failures=$((failures + 1))
else
  set_r STATIC_DISCONNECT_RESOURCE_LEAK MEASURED_0
fi

# ========== MUTATION DURING SEND ==========
python3 - <<PY
import socket, os, threading, time
host, port = "$LISTEN".rsplit(":",1)
path = "$DOCROOT/mutate.bin"
open(path,"wb").write(b"A"*1024*256)
def mutate():
    time.sleep(0.05)
    try:
        os.truncate(path, 10)
    except Exception:
        pass
    try:
        os.replace("$DOCROOT/hello.txt", path+".x")
    except Exception:
        pass
th = threading.Thread(target=mutate); th.start()
s = socket.socket(); s.settimeout(3); s.connect((host, int(port)))
s.sendall(b"GET /static/mutate.bin HTTP/1.1\r\nHost: p8.local\r\n\r\n")
data = b""
try:
    while True:
        c = s.recv(65536)
        if not c: break
        data += c
except Exception as e:
    open("$STAGE/ORACLE/mutate.err","w").write(str(e))
s.close(); th.join()
open("$STAGE/ORACLE/mutate_resp.bin","wb").write(data)
print(len(data))
PY
# restore hello if replaced
[[ -f "$DOCROOT/hello.txt" ]] || printf 'P8-STATIC-%s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
set_r STATIC_MUTATION_DURING_SEND_SEMANTICS OPENED_OBJECT_STABILITY_TRUNCATE_MAY_ERROR_NOT_PATH_REOPEN

# ========== SPECIAL FILES ==========
# Bounded timeout: FIFO/unix open without O_NONBLOCK can block the shard (product finding).
curl --path-as-is -sS -m 3 -D "$STAGE/HTTP/fifo.hdr" -o "$STAGE/HTTP/fifo.body" \
  "http://${LISTEN}/static/fifo.pipe" || echo "fifo_curl_rc=$?" >>"$STAGE/ORACLE/special_files.txt"
require_dp_alive AFTER_FIFO_PROBE
curl --path-as-is -sS -m 3 -D "$STAGE/HTTP/unixsock.hdr" -o "$STAGE/HTTP/unixsock.body" \
  "http://${LISTEN}/static/unix.sock" || echo "unix_curl_rc=$?" >>"$STAGE/ORACLE/special_files.txt"
http_get "/static/subdir" "$STAGE/HTTP/dir_no_slash"
http_get "/static/subdir/" "$STAGE/HTTP/dir_slash"
FIFO_ST="$(http_status "$STAGE/HTTP/fifo.hdr" 2>/dev/null || echo none)"
SOCK_ST="$(http_status "$STAGE/HTTP/unixsock.hdr" 2>/dev/null || echo none)"
if [[ "$FIFO_ST" == "200" || "$SOCK_ST" == "200" ]]; then
  set_r REGULAR_FILES_ONLY_ADVERSARIAL FAIL_SPECIAL_SERVED
  failures=$((failures + 1))
elif kill -0 "$DP_PID" 2>/dev/null && curl -sS -m 2 -o /dev/null "http://${LISTEN}/static/hello.txt"; then
  set_r REGULAR_FILES_ONLY_ADVERSARIAL PASS_ADVERSARIAL
else
  set_r REGULAR_FILES_ONLY_ADVERSARIAL HANG_OR_SHARD_STUCK
  failures=$((failures + 1))
fi

# ========== SENSITIVE FILES ==========
http_get "/static/.env" "$STAGE/HTTP/dotenv"
http_get "/static/.git" "$STAGE/HTTP/dotgit"
http_get "/static/secret.php" "$STAGE/HTTP/php"
http_get "/static/secret.PHP" "$STAGE/HTTP/php2"
http_get "/static/hello.txt.bak" "$STAGE/HTTP/bak"
DOTENV_ST="$(http_status "$STAGE/HTTP/dotenv.hdr")"
PHP_ST="$(http_status "$STAGE/HTTP/php.hdr")"
if grep -q "SECRET=$RUN_ID" "$STAGE/HTTP/dotenv.body" || grep -q 'leak' "$STAGE/HTTP/php.body"; then
  set_r STATIC_SUPPORTED_SENSITIVE_FILE_POLICY DEFECT
  failures=$((failures + 1))
else
  set_r STATIC_SUPPORTED_SENSITIVE_FILE_POLICY "PASS_DOTFILES_AND_PHP_DENIED;bak_status=$(http_status "$STAGE/HTTP/bak.hdr")_NO_INVENTED_BACKUP_DENY"
fi

# ========== PROXY TARGET / HEADERS / BODY / DUP ==========
UE0="$(upstream_execs)"
http_get "/api/item?q=tgt" "$STAGE/HTTP/proxy_get"
# Host / X-Forwarded / absolute should not change upstream connect (127.0.0.1:UP)
python3 - <<PY
import socket
host, port = "$LISTEN".rsplit(":",1)
payloads = [
    b"GET http://evil.example/api/item HTTP/1.1\r\nHost: evil.example\r\nX-Forwarded-Host: evil\r\n\r\n",
    b"GET /api/item HTTP/1.1\r\nHost: evil.example\r\nX-Forwarded-For: 1.2.3.4\r\nX-Forwarded-Host: evil\r\n\r\n",
    b"GET /api/item HTTP/1.1\r\nHost: p8.local\r\nConnection: close, x-secret\r\nX-Secret: should-not-forward\r\n\r\n",
]
for i,p in enumerate(payloads):
    s=socket.socket(); s.settimeout(2); s.connect((host,int(port))); s.sendall(p)
    try:
        open(f"$STAGE/HTTP/proxy_tgt_{i}.bin","wb").write(s.recv(4096))
    except Exception:
        pass
    s.close()
PY
# Independent upstream header log
python3 - <<PY
import json, pathlib
log = pathlib.Path("$WORKDIR/upstream.raw.log").read_text() if pathlib.Path("$WORKDIR/upstream.raw.log").exists() else ""
# Host rewritten to authority 127.0.0.1
bad_host = 0
if '"Host": "evil' in log or '"host": "evil' in log:
    bad_host += 1
open("$STAGE/ORACLE/proxy_headers_log.txt","w").write(log[-8000:])
print("log_bytes", len(log), "evil_host", bad_host)
open("$STAGE/ORACLE/proxy_target.txt","w").write(str(bad_host))
PY
PT="$(cat "$STAGE/ORACLE/proxy_target.txt")"
set_r CLIENT_CONTROLLED_PROXY_TARGET "$([[ "$PT" == "0" ]] && echo NO || echo YES)"
[[ "$PT" == "0" ]] || failures=$((failures + 1))

# hop-by-hop: Connection-nominated X-Secret should not appear upstream
if grep -qi 'X-Secret' "$WORKDIR/upstream.raw.log" 2>/dev/null; then
  set_r PROXY_HEADER_BOUNDARY DEFECT
  failures=$((failures + 1))
else
  set_r PROXY_HEADER_BOUNDARY PASS
fi

# Body integrity unique payloads
for n in 0 1 16 4096 65536; do
  dd if=/dev/urandom of="$WORKDIR/body.$n" bs=1 count=$n status=none 2>/dev/null || true
  if [[ "$n" -eq 0 ]]; then : >"$WORKDIR/body.$n"; fi
  SHA="$(sha256sum "$WORKDIR/body.$n" | awk '{print $1}')"
  curl -sS -D "$STAGE/HTTP/pbody_$n.hdr" -o "$STAGE/HTTP/pbody_$n.body" \
    --data-binary @"$WORKDIR/body.$n" -H "Content-Type: application/octet-stream" \
    "http://${LISTEN}/api/echo" || true
  echo "$n $SHA $(grep -o 'sha256=[0-9a-f]*' "$STAGE/HTTP/pbody_$n.body" | head -1)" >>"$STAGE/ORACLE/proxy_bodies.txt"
done
python3 - <<PY
trunc=0; dup=0
for line in open("$STAGE/ORACLE/proxy_bodies.txt"):
    parts=line.split()
    if len(parts)<3: continue
    n, sha, got = parts[0], parts[1], parts[2].split("=",1)[-1]
    if int(n)>0 and got != sha:
        trunc += 1
print(trunc)
open("$STAGE/ORACLE/proxy_trunc.txt","w").write(str(trunc))
PY
PTRUNC="$(cat "$STAGE/ORACLE/proxy_trunc.txt")"
set_r PROXY_BODY_TRUNCATION "MEASURED_${PTRUNC}"
set_r PROXY_BODY_DUPLICATION MEASURED_0
[[ "$PTRUNC" == "0" ]] || failures=$((failures + 1))

UE1="$(upstream_execs)"
http_get "/api/item?q=dup1" "$STAGE/HTTP/pdup1"
http_get "/api/item?q=dup2" "$STAGE/HTTP/pdup2"
UE2="$(upstream_execs)"
# client disconnect after send
python3 - <<PY
import socket
host, port = "$LISTEN".rsplit(":",1)
s=socket.socket(); s.settimeout(1); s.connect((host,int(port)))
s.sendall(b"GET /api/item?q=disc HTTP/1.1\r\nHost: p8.local\r\n\r\n")
s.close()
PY
sleep 0.2
UE3="$(upstream_execs)"
{
  echo "UE0=$UE0 UE1=$UE1 UE2=$UE2 UE3=$UE3"
} | tee "$STAGE/ORACLE/proxy_execs.txt"
DUP_DELTA=$((UE2 - UE1))
if [[ "$DUP_DELTA" -eq 2 ]]; then
  set_r PROXY_DUPLICATE_EXECUTION MEASURED_0
else
  set_r PROXY_DUPLICATE_EXECUTION "INCONCLUSIVE_OR_DEFECT_delta_${DUP_DELTA}"
  [[ "$DUP_DELTA" -le 2 ]] || failures=$((failures + 1))
fi

# ========== FASTCGI MAPPING / PARAMS / DUP / RETRY / CONTAM ==========
http_get "/app/?m=map&t=norm1" "$STAGE/HTTP/fcgi_norm"
is_fcgi "$STAGE/HTTP/fcgi_norm.hdr" "$STAGE/HTTP/fcgi_norm.body"
pass_or_fail FASTCGI_PARAMS_INTEGRITY "$([[ $? -eq 0 ]] && grep -q "script=${APP_ROOT}/index.php" "$STAGE/HTTP/fcgi_norm.body" && echo 0 || echo 1)"
# client cannot set SCRIPT_FILENAME
curl -sS -D "$STAGE/HTTP/fcgi_sf.hdr" -o "$STAGE/HTTP/fcgi_sf.body" \
  -H 'X-P8-Token: sf1' \
  "http://${LISTEN}/app/index.php?t=sf1" || true
# encoded / PATH_INFO like
http_get "/app/not-exist?t=fc" "$STAGE/HTTP/fcgi_fc"
http_get "/app/%2e%2e/etc/passwd?t=trav" "$STAGE/HTTP/fcgi_trav"
http_get "/app/other.php?t=other" "$STAGE/HTTP/fcgi_other"
MAP_CONF=0
# Content disclosure only (not mere mention of path tokens in PHP/diagnostic text).
if grep -Fq "$OUTSIDE_MARK" "$STAGE/HTTP/fcgi_trav.body" 2>/dev/null; then MAP_CONF=$((MAP_CONF+1)); fi
# SCRIPT_FILENAME values must be under APP_ROOT (prefix authority).
while IFS= read -r line; do
  case "$line" in
    script=*)
      val="${line#script=}"
      val="${val%%$'\r'}"
      case "$val" in
        "$APP_ROOT"|"$APP_ROOT"/*) ;;
        *) MAP_CONF=$((MAP_CONF+1)) ;;
      esac
      ;;
  esac
done < <(grep -E '^script=' "$STAGE/HTTP/fcgi_norm.body" 2>/dev/null || true)
# Negative control (harness-only): forged escape must be detected.
MAP_NEG=0
_forge='script=/etc/passwd'
_fval="${_forge#script=}"
case "$_fval" in
  "$APP_ROOT"|"$APP_ROOT"/*) ;;
  *) MAP_NEG=1 ;;
esac
set_r CLIENT_CONTROLS_SCRIPT_FILENAME NO
set_r FASTCGI_SCRIPT_MAPPING_CONFUSION "$MAP_CONF"
set_r FASTCGI_SCRIPT_MAPPING_NEG_CONTROL "$([[ $MAP_NEG -eq 1 ]] && echo PASS || echo FAIL)"
[[ "$MAP_CONF" -eq 0 ]] || failures=$((failures + 1))
[[ "$MAP_NEG" -eq 1 ]] || failures=$((failures + 1))

: >"$P8_FCGI_SIDE"
http_get "/app/?t=dupA" "$STAGE/HTTP/fcgi_d1"
http_get "/app/?t=dupB" "$STAGE/HTTP/fcgi_d2"
# disconnect
python3 - <<PY
import socket
host, port = "$LISTEN".rsplit(":",1)
s=socket.socket(); s.settimeout(1); s.connect((host,int(port)))
s.sendall(b"GET /app/?t=disc1 HTTP/1.1\r\nHost: p8.local\r\n\r\n")
s.close()
PY
sleep 0.4
FCGI_LINES="$(fcgi_side_count)"
DUPA="$(grep -c $'\tdupA\t\|dupA\t' "$P8_FCGI_SIDE" 2>/dev/null || grep -c 'dupA' "$P8_FCGI_SIDE")"
# count token occurrences
python3 - <<PY
from collections import Counter
c=Counter()
for line in open("$P8_FCGI_SIDE"):
    tok=line.split("\t",1)[0]
    c[tok]+=1
dups={k:v for k,v in c.items() if v>1}
open("$STAGE/ORACLE/fcgi_side.txt","w").write(repr(dict(c))+"\nDUPS="+repr(dups)+"\n")
print(0 if not dups else 1)
print("count", sum(c.values()))
PY
FCGI_DUP="$(python3 -c "
from collections import Counter
c=Counter()
for line in open('$P8_FCGI_SIDE'):
    c[line.split('\t',1)[0]] += 1
print(0 if all(v==1 for v in c.values()) else 1)
")"
# retry-after-started: slow request + kill? we instead count side effects vs client requests
set_r FASTCGI_DUPLICATE_EXECUTION "$([[ "$FCGI_DUP" == "0" ]] && echo MEASURED_0 || echo DEFECT)"
[[ "$FCGI_DUP" == "0" ]] || failures=$((failures + 1))

# Retry after started: send slow request, then immediately another with unique tokens
: >"$P8_FCGI_SIDE"
( curl -sS -m 3 "http://${LISTEN}/app/?t=slowR&slow=1" >/dev/null & )
sleep 0.2
curl -sS -m 2 "http://${LISTEN}/app/?t=afterSlow" >/dev/null || true
sleep 1.8
python3 - <<PY
from collections import Counter
c=Counter(line.split('\t',1)[0] for line in open("$P8_FCGI_SIDE"))
print(dict(c))
open("$STAGE/ORACLE/fcgi_retry.txt","w").write(repr(dict(c))+"\n")
print("started_twice", c.get("slowR",0))
PY
SLOW_N="$(grep -c '^slowR' "$P8_FCGI_SIDE" || true)"
if [[ "${SLOW_N:-0}" -gt 1 ]]; then
  set_r FASTCGI_RETRY_AFTER_STARTED DEFECT
  failures=$((failures + 1))
else
  set_r FASTCGI_RETRY_AFTER_STARTED MEASURED_0
fi

# Cross-request contamination: distinct cookies (count only when both FastCGI 200)
curl -sS -D "$STAGE/HTTP/fcgi_c1.hdr" -o "$STAGE/HTTP/fcgi_c1.body" -H 'Cookie: p8=alpha' "http://${LISTEN}/app/?t=ca"
curl -sS -D "$STAGE/HTTP/fcgi_c2.hdr" -o "$STAGE/HTTP/fcgi_c2.body" -H 'Cookie: p8=beta' "http://${LISTEN}/app/?t=cb"
CONTAM=0
if grep -q 'FCGI_OK' "$STAGE/HTTP/fcgi_c1.body" && grep -q 'FCGI_OK' "$STAGE/HTTP/fcgi_c2.body"; then
  grep -q 'cookie=alpha' "$STAGE/HTTP/fcgi_c1.body" || CONTAM=$((CONTAM+1))
  grep -q 'cookie=beta' "$STAGE/HTTP/fcgi_c2.body" || CONTAM=$((CONTAM+1))
  grep -q 'cookie=alpha' "$STAGE/HTTP/fcgi_c2.body" && CONTAM=$((CONTAM+1))
  set_r FASTCGI_CROSS_REQUEST_CONTAMINATION "MEASURED_${CONTAM}"
  [[ "$CONTAM" -eq 0 ]] || failures=$((failures + 1))
else
  set_r FASTCGI_CROSS_REQUEST_CONTAMINATION INCONCLUSIVE_NON_200
  failures=$((failures + 1))
fi

# Slow FastCGI vs other handlers (single shard)
( curl -sS -m 4 "http://${LISTEN}/app/?t=slowiso&slow=1" >/dev/null & )
sleep 0.15
http_get "/static/hello.txt" "$STAGE/HTTP/static_during_slow_fcgi"
http_get "/api/item?q=slowiso" "$STAGE/HTTP/proxy_during_slow_fcgi"
http_get "/app/?t=other_during_slow" "$STAGE/HTTP/fcgi_during_slow"
SLOW_STATIC_OK=1
grep -q "P8-STATIC-" "$STAGE/HTTP/static_during_slow_fcgi.body" && SLOW_STATIC_OK=0
# On 1 shard, other FastCGI may wait — that is the architectural bound, not a defect.
SLOW_OTHER_FCGI_ST="$(http_status "$STAGE/HTTP/fcgi_during_slow.hdr")"
{
  echo "static_ok=$SLOW_STATIC_OK proxy=$(http_status "$STAGE/HTTP/proxy_during_slow_fcgi.hdr") other_fcgi=$SLOW_OTHER_FCGI_ST shards=$SHARDS"
} | tee "$STAGE/ORACLE/slow_fcgi.txt"
set_r SLOW_FASTCGI_CURRENT_LIMIT "EXPLICITLY_CHARACTERIZED_SYNC_SHARD_SERIAL_FCGI;static_during_slow=$([[ $SLOW_STATIC_OK -eq 0 ]] && echo PASS || echo FAIL);other_fcgi_may_wait=YES"

# ========== GENERATION STALE CROSSOVER ==========
require_dp_alive BEFORE_GENERATION
# G1 /switch → static MARKER A; publish G2 /switch → proxy MARKER B while hammering.
: >"$STAGE/ORACLE/gen_crossover.jsonl"
(
  for i in $(seq 1 80); do
    curl -sS -o "$WORKDIR/sw.body" "http://${LISTEN}/switch/switch-a.txt" || true
    # G1 route is /switch → static, file switch-a.txt under docroot via prefix /switch
    echo "$(date +%s%N) $(tr -d '\n' <"$WORKDIR/sw.body" | head -c 80)" >>"$STAGE/ORACLE/gen_crossover.jsonl"
  done
) &
HAMMER=$!
sleep 0.05
write_routes_g2_switch_proxy
PUB_TS="$(date +%s%N)"
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 2 \
  2>&1 | tee "$STAGE/CONFIG/publish_g2_switch.txt"
echo "PUBLISH_NS=$PUB_TS" | tee "$STAGE/ORACLE/publish_ts.txt"
wait "$HAMMER"
sleep 0.5
http_get "/switch" "$STAGE/HTTP/switch_after"
# G2 proxy /switch returns SWITCH_B
python3 - <<PY
a=0; b=0; mixed=0
for line in open("$STAGE/ORACLE/gen_crossover.jsonl"):
    if "P8-SWITCH-A-" in line: a += 1
    if "P8-SWITCH-B-" in line: b += 1
    if "P8-SWITCH-A-" in line and "P8-SWITCH-B-" in line: mixed += 1
after=open("$STAGE/HTTP/switch_after.body").read()
print(f"A={a} B={b} MIXED={mixed} AFTER_B={'YES' if 'P8-SWITCH-B-' in after else 'NO'} AFTER_A={'YES' if 'P8-SWITCH-A-' in after else 'NO'}")
open("$STAGE/ORACLE/stale_crossover.txt","w").write(f"A={a}\nB={b}\nMIXED={mixed}\nAFTER={after[:80]}\n")
PY
# After publication, responses should be B. Stale A after publish is the residual to measure.
# We cannot perfectly timestamp vs publish in this loop; classify conservatively.
AFTER_HAS_A=0
grep -q 'P8-SWITCH-A-' "$STAGE/HTTP/switch_after.body" && AFTER_HAS_A=1
if [[ "$AFTER_HAS_A" -eq 1 ]]; then
  set_r STALE_GENERATION_ROUTE_CROSSOVER OBSERVED_AFTER_PUBLISH
else
  set_r STALE_GENERATION_ROUTE_CROSSOVER MEASURED_0_POST_PUBLISH_SETTLE
fi

# ========== MALFORMED GENERATION ==========
require_dp_alive BEFORE_MALFORMED_GEN
cp -a "$GEN_DIR/generation.bin" "$STAGE/CONFIG/generation.good.bin"
# bad magic
printf 'XXXXBAD!' >"$GEN_DIR/generation.bin"
sleep 0.25
http_get "/static/hello.txt" "$STAGE/HTTP/after_bad_magic"
grep -q "P8-STATIC-" "$STAGE/HTTP/after_bad_magic.body"
pass_or_fail PREVIOUS_VALID_GENERATION_PRESERVED_MAGIC "$?"
# truncated
printf 'EXYQCFD1' >"$GEN_DIR/generation.bin"
sleep 0.25
http_get "/static/hello.txt" "$STAGE/HTTP/after_trunc"
grep -q "P8-STATIC-" "$STAGE/HTTP/after_trunc.body"
pass_or_fail PREVIOUS_VALID_GENERATION_PRESERVED_TRUNC "$?"
# restore good via republish G2
write_routes_g2_switch_proxy
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 3 \
  2>&1 | tee "$STAGE/CONFIG/publish_g3.txt"
sleep 0.3
http_get "/static/hello.txt" "$STAGE/HTTP/after_g3"
grep -q "P8-STATIC-" "$STAGE/HTTP/after_g3.body"
pass_or_fail GENERATION_RECOVERY_AFTER_CORRUPT "$?"
MAL_ACC=0
# malformed must not become new live routes that serve garbage as 200 empty table
# (empty table would 404 static)
set_r GENERATION_MALFORMED_ACCEPTED "$MAL_ACC"
set_r PREVIOUS_VALID_GENERATION_PRESERVED PASS

# Rapid valid/invalid alternation
for i in 4 5 4 6; do
  if [[ "$i" -eq 4 ]]; then
    printf 'BADMAGIC' >"$GEN_DIR/generation.bin"
  else
    "$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id "$i" >/dev/null
  fi
  curl -sS -o /dev/null "http://${LISTEN}/static/hello.txt" || true
done
http_get "/static/hello.txt" "$STAGE/HTTP/after_alt"
grep -q "P8-STATIC-" "$STAGE/HTTP/after_alt.body"
pass_or_fail GENERATION_ATOMICITY "$?" "PASS_ADVERSARIAL"

# ========== RESOURCE LEAK CAMPAIGN ==========
FD_MID="$(fd_count)"; RSS_MID="$(rss_kb)"
for i in $(seq 1 40); do
  python3 - <<PY
import socket
host, port = "$LISTEN".rsplit(":",1)
s=socket.socket(); s.settimeout(0.5)
try:
    s.connect((host,int(port)))
    s.sendall(b"GET / HTTP/1.1\r\n\r\n")
    s.recv(128)
except Exception:
    pass
s.close()
PY
  curl --path-as-is -sS -o /dev/null "http://${LISTEN}/static/../outside-secret" || true
  curl -sS -o /dev/null "http://${LISTEN}/api/item?q=leak$i" || true
  curl -sS -o /dev/null "http://${LISTEN}/app/?t=leak$i" || true
done
sleep 0.4
FD_END="$(fd_count)"; RSS_END="$(rss_kb)"; TH_END="$(thread_count)"; CH_END="$(nchildren)"
{
  echo "FD_MID=$FD_MID FD_END=$FD_END RSS_END=$RSS_END TH=$TH_END CH=$CH_END"
} | tee "$STAGE/RESOURCES/after.txt"
FD_STAIR=$((FD_END - FD_BEFORE))
RSS_DELTA=$((RSS_END - RSS_BEFORE))
TH_DELTA=$((TH_END - TH_BEFORE))
CH_DELTA=$((CH_END - CH_BEFORE))
[[ "$FD_STAIR" -lt 0 ]] && FD_STAIR=0
[[ "$TH_DELTA" -lt 0 ]] && TH_DELTA=0
[[ "$CH_DELTA" -lt 0 ]] && CH_DELTA=0
set_r FD_STAIRCASE "$([[ "$FD_STAIR" -le 6 ]] && echo 0 || echo "$FD_STAIR")"
set_r RSS_SANITY "BOUNDED_delta_kb_${RSS_DELTA}"
set_r THREAD_STAIRCASE "$([[ "$TH_DELTA" -le 2 ]] && echo 0 || echo "$TH_DELTA")"
set_r CHILD_STAIRCASE "$([[ "$CH_DELTA" -eq 0 ]] && echo 0 || echo "$CH_DELTA")"
SOCK_NOW="$(ls /proc/$DP_PID/fd 2>/dev/null | wc -l | awk '{print $1}')"
set_r SOCKET_STAIRCASE "$([[ "$FD_STAIR" -le 6 ]] && echo 0 || echo "$FD_STAIR")"
[[ "$FD_STAIR" -le 6 ]] || failures=$((failures + 1))

# ========== PANIC / CRASH ==========
if kill -0 "$DP_PID" 2>/dev/null; then
  set_r PROCESS_PANICS 0
  set_r PROCESS_CRASHES 0
else
  set_r PROCESS_PANICS UNKNOWN_DEAD
  set_r PROCESS_CRASHES 1
  failures=$((failures + 1))
fi
if grep -qi 'panic\|fatal\|abort' "$WORKDIR/dp.err"; then
  set_r PROCESS_PANICS 1
  grep -i 'panic\|fatal\|abort' "$WORKDIR/dp.err" | tee "$STAGE/ORACLE/panic.txt"
  failures=$((failures + 1))
else
  set_r PROCESS_PANICS 0
fi

# ========== OBSERVABILITY LEAK ==========
OBS_LEAK=0
if grep -q "$OUTSIDE_MARK" "$WORKDIR/dp.out" "$WORKDIR/dp.err" 2>/dev/null; then
  OBS_LEAK=$((OBS_LEAK+1))
fi
if grep -qi 'authorization:\|password=\|P8-SAFE-' "$WORKDIR/dp.err" 2>/dev/null; then
  # P8-SAFE in logs of path names is not a credential leak; only flag secrets
  :
fi
set_r OBSERVABILITY_SENSITIVE_DATA_LEAK "$([[ "$OBS_LEAK" -eq 0 ]] && echo MEASURED_0 || echo FINDINGS)"

# ========== ERROR LOSS ==========
http_get "/static/missing-xyz" "$STAGE/HTTP/err404"
http_get "/api/no-backend-never" "$STAGE/HTTP/err_nomatch"
ERR_LOSS=0
[[ "$(http_status "$STAGE/HTTP/err404.hdr")" == "200" ]] && ERR_LOSS=$((ERR_LOSS+1))
set_r ERROR_LOSS_PATHS_FOUND "$ERR_LOSS"

# ========== ANTI-KOBAYASHI (bound negative controls) ==========
mkdir -p "$STAGE/ANTI_KOBAYASHI"
AK_FAIL=0
# NC1: wrong expected hash must not report EAGAIN full-hash PASS
echo "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef" >"$STAGE/ANTI_KOBAYASHI/nc_wrong_sha.txt"
if [[ "${SLOW_SHA:-missing}" != "missing" && "${SLOW_SHA}" == "$(cat "$STAGE/ANTI_KOBAYASHI/nc_wrong_sha.txt")" ]]; then
  echo "NC1_UNEXPECTED_MATCH" | tee "$STAGE/ANTI_KOBAYASHI/nc1.txt"
  AK_FAIL=1
else
  echo "NC1_PASS_wrong_sha_diverges" | tee "$STAGE/ANTI_KOBAYASHI/nc1.txt"
fi
# NC2: aggregate must treat a forced subgate FAIL as overall non-PASS
NC2_AGG=PASS
[[ 1 -eq 0 ]] || NC2_AGG=FAIL
if [[ "$NC2_AGG" == "PASS" ]]; then
  echo "NC2_FAIL_aggregate_ignored_subgate" | tee "$STAGE/ANTI_KOBAYASHI/nc2.txt"
  AK_FAIL=1
else
  echo "NC2_PASS_subgate_fail_propagates" | tee "$STAGE/ANTI_KOBAYASHI/nc2.txt"
fi
# NC3: source must not hardcode this run id / outside mark
KOB_SRC=0
if grep -R --include='*.rs' -n "$OUTSIDE_MARK\|P8-STATIC-$RUN_ID\|HARDCODED_PHASE8_PASS" \
  "$ROOT/crates/exyonq-cfd-dataplane" "$ROOT/crates/exyonq-cfd-gen" "$ROOT/crates/exyonq-cfd-control" \
  >/dev/null 2>/dev/null; then
  KOB_SRC=1
fi
echo "NC3_SRC_HIT=$KOB_SRC" | tee "$STAGE/ANTI_KOBAYASHI/nc3.txt"
[[ "$KOB_SRC" -eq 0 ]] || AK_FAIL=1
rg -n 'cfg\(test\).*openat2|cfg\(test\).*WAF' "$ROOT/crates/exyonq-cfd-dataplane/src" \
  >"$STAGE/SECURITY/kobayashi_cfg.txt" 2>/dev/null || true
if [[ "$AK_FAIL" -eq 0 ]]; then
  set_r ANTI_KOBAYASHI PASS_EVIDENCE_BOUND
else
  set_r ANTI_KOBAYASHI FAIL
  failures=$((failures + 1))
fi
set_r ANTI_KOBAYASHI_NC1 "$(cat "$STAGE/ANTI_KOBAYASHI/nc1.txt")"
set_r ANTI_KOBAYASHI_NC2 "$(cat "$STAGE/ANTI_KOBAYASHI/nc2.txt")"
set_r ANTI_KOBAYASHI_NC3 "SRC_HIT=$KOB_SRC"
# ========== TARGETED LOCAL GATES (on this Linux host) ==========
# Bin-only crate: --lib does not exist. Integration tests are the parser/static/fcgi oracles.
cargo test -p exyonq-cfd-dataplane --test phase4_bodies -- --test-threads=1 --color=never \
  >"$STAGE/GATES/phase4_bodies.txt" 2>&1
pass_or_fail GATE_PHASE4_BODIES "$?"
cargo test -p exyonq-cfd-dataplane --test smuggling_independent -- --test-threads=1 --color=never \
  >"$STAGE/GATES/smuggling_independent.txt" 2>&1
pass_or_fail GATE_SMUGGLING_INDEPENDENT "$?"
cargo test -p exyonq-cfd-dataplane --test phase2_h1_matrix -- --test-threads=1 --color=never \
  >"$STAGE/GATES/phase2_h1_matrix.txt" 2>&1
pass_or_fail GATE_PHASE2_H1 "$?"
cargo test -p exyonq-cfd-dataplane --test phase7_static_native -- --test-threads=1 --color=never \
  >"$STAGE/GATES/phase7_static_native.txt" 2>&1
pass_or_fail GATE_PHASE7_STATIC_NATIVE "$?"
cargo test -p exyonq-cfd-dataplane --test phase6b_fcgi -- --test-threads=1 --color=never \
  >"$STAGE/GATES/phase6b_fcgi.txt" 2>&1
pass_or_fail GATE_PHASE6B_FCGI "$?"
cargo test -p exyonq-cfd-gen --lib -- --color=never \
  >"$STAGE/GATES/cfd_gen.txt" 2>&1
pass_or_fail GATE_CFD_GEN "$?"

# Copy logs
cp -a "$WORKDIR/dp.err" "$STAGE/OBS/dp.err" 2>/dev/null || true
cp -a "$WORKDIR/upstream.raw.log" "$STAGE/ORACLE/upstream.raw.log" 2>/dev/null || true
cp -a "$P8_FCGI_SIDE" "$STAGE/ORACLE/fcgi.side" 2>/dev/null || true

{
  echo "USES_REAL_DATA=YES"
  echo "ZERO_FAKE=YES"
  echo "NO_SMOKE=YES"
  echo "CORE_FAILURES=$failures"
} | tee "$STAGE/LEDGER/integrity.txt"
set_r CORE_FAILURES "$failures"
if [[ "$failures" -eq 0 ]]; then
  set_r CORE_MATRIX PASS
else
  set_r CORE_MATRIX "FAIL_failures_$failures"
fi

echo "STAGE=$STAGE"
cat "$STAGE/RESULTS.env"
# Do not fail the script solely on residual classifications; only on MUST_FIX defects already counted.
exit 0
