#!/usr/bin/env bash
# V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_IMPLEMENTATION — Netcup amd64 reality harness.
# PARENT=P7STATICADM-B. This is not Phase 7 requalification.
#
# Route mapping is intentional and matches static_serve::resolve_relative_path:
#   route /static + request /static/hello.txt => docroot/hello.txt
#   route /assets + request /assets/binary.bin => docroot/binary.bin
#   route /static + request /static/subdir/ => docroot/subdir/index.html
#
# PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/cfd-native-static-filesystem-handler-implementation}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
WORKDIR="${CFD_STATIC_WORKDIR:-/tmp/exyonq-cfd-static-$RUN_ID}"
DOCROOT="$WORKDIR/docroot"
OUTSIDE="$WORKDIR/outside-secret"
GEN_DIR="$WORKDIR/gen"
PORT_TAG=$((0x$(printf '%s' "$RUN_ID" | sha256sum | cut -c 1-3) % 2000))
LISTEN="${LISTEN:-127.0.0.1:$((18440 + PORT_TAG))}"
SHARDS="${SHARDS:-1}"

DP_PID=""
cleanup() {
  local rc=$?
  if [[ -n "$DP_PID" ]]; then
    kill "$DP_PID" 2>/dev/null || true
    wait "$DP_PID" 2>/dev/null || true
  fi
  date -u +"END=%Y-%m-%dT%H:%M:%SZ" >>"$STAGE/manifest.txt" 2>/dev/null || true
  exit "$rc"
}
trap cleanup EXIT

mkdir -p "$STAGE"/{SOURCE,BUILD,HOST,CONFIG,HTTP,ORACLE,RESOURCES,STRACE,LEDGER} \
  "$DOCROOT/subdir" "$GEN_DIR"
: >"$STAGE/RESULTS.env"
set_r() { printf '%s=%s\n' "$1" "$2" | tee -a "$STAGE/RESULTS.env"; }
failures=0
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
  echo "WIP=V044_CFD_NATIVE_STATIC_FILESYSTEM_HANDLER_IMPLEMENTATION"
  echo "PARENT=P7STATICADM-B"
  echo "PLATFORM=LINUX_AMD64_NETCUP"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  echo "EXYONQ_ROOT=$ROOT"
  echo "LISTEN=$LISTEN"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

git -C "$ROOT" status --short | tee "$STAGE/SOURCE/AMBIENT_DIRT_DECLARATION.txt" || true
sha256sum \
  "$ROOT/crates/exyonq-cfd-dataplane/src/static_serve.rs" \
  "$ROOT/crates/exyonq-cfd-dataplane/src/shard.rs" \
  "$ROOT/crates/exyonq-cfd-control/src/project.rs" \
  "$ROOT/crates/exyonq-cfd-gen/src/route_table.rs" \
  | tee "$STAGE/SOURCE/file_sha256.txt"

{
  hostname
  uname -a
  echo "UNAME_M=$(uname -m)"
  rustc --version
  cargo --version
  command -v strace >/dev/null && strace -V | awk 'NR==1 {print; exit}' || true
} | tee "$STAGE/HOST/identity.txt"

if [[ "$(uname -m)" != "x86_64" ]]; then
  set_r PLATFORM_CHECK FAIL_NOT_AMD64
  exit 2
fi
set_r PLATFORM_CHECK PASS_LINUX_AMD64_NETCUP

echo "[cfd-static] build release dataplane + route publisher"
cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
  2>&1 | tee "$STAGE/BUILD/cargo-build.log"
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { set_r BUILD_BINARIES FAIL; exit 3; }
sha256sum "$BIN" "$PUB" | tee "$STAGE/BUILD/binary_sha256.txt"
set_r BUILD_BINARIES PASS

printf 'hello static %s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
dd if=/dev/urandom of="$DOCROOT/binary.bin" bs=4096 count=8 status=none
dd if=/dev/urandom of="$DOCROOT/large.bin" bs=1048576 count=2 status=none
printf '<!doctype html><title>CFD static %s</title>\n' "$RUN_ID" >"$DOCROOT/subdir/index.html"
printf 'EXYONQ_SECRET_SHOULD_NOT_LEAK=%s\n' "$RUN_ID" >"$DOCROOT/.env"
printf '<?php echo "secret"; ?>\n' >"$DOCROOT/secret.php"
printf 'outside secret %s\n' "$RUN_ID" >"$OUTSIDE"
rm -f "$DOCROOT/outside-link"
ln -sf "$OUTSIDE" "$DOCROOT/outside-link"

sha256sum "$DOCROOT/hello.txt" "$DOCROOT/binary.bin" "$DOCROOT/large.bin" \
  "$DOCROOT/subdir/index.html" "$DOCROOT/.env" "$DOCROOT/secret.php" "$OUTSIDE" \
  | tee "$STAGE/CONFIG/fixture_sha256.txt"
HELLO_SHA="$(sha256sum "$DOCROOT/hello.txt" | awk '{print $1}')"
BINARY_SHA="$(sha256sum "$DOCROOT/binary.bin" | awk '{print $1}')"
LARGE_SHA="$(sha256sum "$DOCROOT/large.bin" | awk '{print $1}')"
INDEX_SHA="$(sha256sum "$DOCROOT/subdir/index.html" | awk '{print $1}')"
OUTSIDE_SHA="$(sha256sum "$OUTSIDE" | awk '{print $1}')"
set_r HELLO_SHA256 "$HELLO_SHA"
set_r BINARY_SHA256 "$BINARY_SHA"
set_r LARGE_SHA256 "$LARGE_SHA"
set_r INDEX_SHA256 "$INDEX_SHA"
set_r OUTSIDE_SHA256 "$OUTSIDE_SHA"

cat >"$WORKDIR/routes.txt" <<EOF
# CFDRT005 native static policy. Requests under /static and /assets strip that
# route prefix before resolving under DOCROOT.
static|1|${DOCROOT}|index.html
|/static|static:1
|/assets|static:1
EOF
cp -a "$WORKDIR/routes.txt" "$STAGE/CONFIG/routes.txt"
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 1 \
  2>&1 | tee "$STAGE/CONFIG/publish.txt"
set_r ROUTE_PUBLISH PASS

"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$SHARDS" --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
echo "$DP_PID" | tee "$STAGE/RESOURCES/dp.pid"

ready=0
for _ in $(seq 1 80); do
  if curl -sS -o /dev/null "http://$LISTEN/static/hello.txt"; then
    ready=1
    break
  fi
  sleep 0.1
done
cp -a "$WORKDIR/dp.out" "$STAGE/RESOURCES/dp.out" 2>/dev/null || true
cp -a "$WORKDIR/dp.err" "$STAGE/RESOURCES/dp.err" 2>/dev/null || true
if [[ "$ready" -ne 1 ]]; then
  set_r DATAPLANE_READY FAIL
  exit 4
fi
set_r DATAPLANE_READY PASS

# From here, collect the whole matrix and summarize failures in RESULTS.env.
set +e

fd_count() { ls "/proc/$DP_PID/fd" 2>/dev/null | wc -l | awk '{print $1}'; }
rss_kb() { awk '/^VmRSS:/ {print $2; found=1} END {if (!found) print 0}' "/proc/$DP_PID/status" 2>/dev/null; }
FD_BEFORE="$(fd_count)"
RSS_BEFORE="$(rss_kb)"
{
  echo "DP_PID=$DP_PID"
  echo "FD_BEFORE=$FD_BEFORE"
  echo "RSS_BEFORE_KB=$RSS_BEFORE"
  ps -o pid,rss,nlwp,cmd -p "$DP_PID" || true
} | tee "$STAGE/RESOURCES/before.txt"
set_r FD_BEFORE "$FD_BEFORE"
set_r RSS_BEFORE_KB "$RSS_BEFORE"

http_get() {
  local path="$1" out="$2"
  curl --path-as-is -sS -D "$out.hdr" -o "$out.body" "http://${LISTEN}${path}" || true
}
http_status() { awk 'NR==1 {print $2; exit}' "$1"; }
body_sha() { sha256sum "$1" | awk '{print $1}'; }
header_value() {
  local name="$1" hdr="$2"
  awk -v n="$name" 'BEGIN{IGNORECASE=1} index($0,n ":")==1 {sub(/^[^:]*:[[:space:]]*/,""); sub(/\r$/,""); print; exit}' "$hdr"
}

http_get "/static/hello.txt" "$STAGE/HTTP/get_small"
small_status="$(http_status "$STAGE/HTTP/get_small.hdr")"
small_sha="$(body_sha "$STAGE/HTTP/get_small.body")"
[[ "$small_status" == "200" && "$small_sha" == "$HELLO_SHA" ]]
pass_or_fail GET_SMALL "$?" "status_${small_status}_sha_${small_sha}"

http_get "/assets/binary.bin" "$STAGE/HTTP/get_binary"
binary_status="$(http_status "$STAGE/HTTP/get_binary.hdr")"
binary_sha="$(body_sha "$STAGE/HTTP/get_binary.body")"
[[ "$binary_status" == "200" && "$binary_sha" == "$BINARY_SHA" ]]
pass_or_fail GET_BINARY "$?" "status_${binary_status}_sha_${binary_sha}"

http_get "/static/large.bin" "$STAGE/HTTP/get_large"
large_status="$(http_status "$STAGE/HTTP/get_large.hdr")"
large_sha="$(body_sha "$STAGE/HTTP/get_large.body")"
[[ "$large_status" == "200" && "$large_sha" == "$LARGE_SHA" ]]
pass_or_fail GET_LARGE "$?" "status_${large_status}_sha_${large_sha}"

curl -sS -I "http://${LISTEN}/static/hello.txt" >"$STAGE/HTTP/head_small.hdr" || true
head_status="$(http_status "$STAGE/HTTP/head_small.hdr")"
head_len="$(header_value Content-Length "$STAGE/HTTP/head_small.hdr")"
file_len="$(wc -c <"$DOCROOT/hello.txt" | awk '{print $1}')"
[[ "$head_status" == "200" && "$head_len" == "$file_len" ]]
pass_or_fail HEAD_SMALL "$?" "status_${head_status}_len_${head_len}"

txt_type="$(header_value Content-Type "$STAGE/HTTP/get_small.hdr")"
bin_type="$(header_value Content-Type "$STAGE/HTTP/get_binary.hdr")"
[[ "$txt_type" == "text/plain; charset=utf-8" && "$bin_type" == "application/octet-stream" ]]
pass_or_fail MIME "$?" "txt_${txt_type// /_}_bin_${bin_type// /_}"

http_get "/static/subdir/" "$STAGE/HTTP/get_index"
index_status="$(http_status "$STAGE/HTTP/get_index.hdr")"
index_sha="$(body_sha "$STAGE/HTTP/get_index.body")"
[[ "$index_status" == "200" && "$index_sha" == "$INDEX_SHA" ]]
pass_or_fail DIRECTORY_INDEX "$?" "status_${index_status}_sha_${index_sha}"

for case_name in missing dotenv php traverse traverse_encoded symlink_out; do
  case "$case_name" in
    missing) path="/static/missing.txt"; expect=404 ;;
    dotenv) path="/static/.env"; expect=403 ;;
    php) path="/static/secret.php"; expect=403 ;;
    traverse) path="/static/../outside-secret"; expect=403 ;;
    traverse_encoded) path="/static/%2e%2e/outside-secret"; expect=403 ;;
    symlink_out) path="/static/outside-link"; expect=403 ;;
  esac
  out="$STAGE/HTTP/$case_name"
  http_get "$path" "$out"
  st="$(http_status "$out.hdr")"
  sha="$(body_sha "$out.body")"
  [[ "$st" == "$expect" && "$sha" != "$OUTSIDE_SHA" ]]
  pass_or_fail "HTTP_${case_name^^}" "$?" "status_${st}"
done

# Sequential keepalive (not HTTP pipelining — foundation rejects pipelined bytes).
python3 - "$LISTEN" "$HELLO_SHA" "$BINARY_SHA" "$STAGE/HTTP/keepalive_two_gets.txt" <<'PY'
import hashlib
import socket
import sys

listen, hello_sha, binary_sha, out_path = sys.argv[1:5]
host, port_s = listen.rsplit(":", 1)
sock = socket.create_connection((host, int(port_s)), timeout=5)
sock.settimeout(5)

def read_one(sock):
    buf = b""
    while True:
        chunk = sock.recv(65536)
        if not chunk:
            break
        buf += chunk
        head_end = buf.find(b"\r\n\r\n")
        if head_end < 0:
            continue
        head = buf[:head_end].decode("iso-8859-1")
        length = 0
        for line in head.split("\r\n")[1:]:
            if line.lower().startswith("content-length:"):
                length = int(line.split(":", 1)[1].strip())
                break
        need = head_end + 4 + length
        if len(buf) >= need:
            return head, buf[head_end + 4 : need], buf[need:]
    raise RuntimeError("incomplete response")

sock.sendall(
    b"GET /static/hello.txt HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n"
)
h1, b1, rest = read_one(sock)
if rest:
    raise RuntimeError("unexpected leftover after first response")
sock.sendall(
    b"GET /assets/binary.bin HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
)
h2, b2, rest = read_one(sock)
sock.close()
ok = (
    h1.startswith("HTTP/1.1 200 ")
    and h2.startswith("HTTP/1.1 200 ")
    and hashlib.sha256(b1).hexdigest() == hello_sha
    and hashlib.sha256(b2).hexdigest() == binary_sha
    and rest == b""
)
with open(out_path, "wb") as f:
    f.write(b"FIRST_HEAD\n" + h1.encode() + b"\nFIRST_SHA=" + hashlib.sha256(b1).hexdigest().encode() + b"\n")
    f.write(b"SECOND_HEAD\n" + h2.encode() + b"\nSECOND_SHA=" + hashlib.sha256(b2).hexdigest().encode() + b"\n")
    f.write(b"REST_LEN=" + str(len(rest)).encode() + b"\n")
sys.exit(0 if ok else 1)
PY
pass_or_fail KEEPALIVE_TWO_GETS "$?"

for _ in $(seq 1 50); do
  curl -sS -o /dev/null "http://${LISTEN}/static/hello.txt"
done
FD_AFTER="$(fd_count)"
RSS_AFTER="$(rss_kb)"
FD_DELTA=$((FD_AFTER - FD_BEFORE))
RSS_DELTA=$((RSS_AFTER - RSS_BEFORE))
{
  echo "DP_PID=$DP_PID"
  echo "FD_AFTER=$FD_AFTER"
  echo "RSS_AFTER_KB=$RSS_AFTER"
  echo "FD_DELTA=$FD_DELTA"
  echo "RSS_DELTA_KB=$RSS_DELTA"
  ps -o pid,rss,nlwp,cmd -p "$DP_PID" || true
} | tee "$STAGE/RESOURCES/after_50.txt"
set_r FD_AFTER "$FD_AFTER"
set_r RSS_AFTER_KB "$RSS_AFTER"
set_r FD_DELTA "$FD_DELTA"
set_r RSS_DELTA_KB "$RSS_DELTA"
[[ "$FD_DELTA" -le 3 ]]
pass_or_fail FD_SANITY "$?" "delta_${FD_DELTA}"
[[ "$RSS_DELTA" -le 8192 ]]
pass_or_fail RSS_SANITY "$?" "delta_kb_${RSS_DELTA}"

# Dedicated short-lived dataplane under strace — avoid attaching to the matrix PID
# (ptrace -f on a live mio process has killed accept in prior runs).
if command -v strace >/dev/null; then
  SF_PORT="$(python3 - <<'PY'
import socket
s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()
PY
)"
  SF_LISTEN="127.0.0.1:${SF_PORT}"
  SF_GEN="$WORKDIR/sf-gen"
  mkdir -p "$SF_GEN" "$STAGE/STRACE"
  cp -a "$GEN_DIR"/. "$SF_GEN"/
  # Run dataplane as strace's child so sendfile syscalls are captured without -p attach.
  timeout 25s strace -f -e sendfile -o "$STAGE/STRACE/large-sendfile.strace" \
    "$BIN" serve --listen "$SF_LISTEN" --gen-dir "$SF_GEN" --shards 1 --schema-version 2 \
    >"$STAGE/STRACE/sf.out" 2>"$STAGE/STRACE/sf.err" &
  SF_PID=$!
  sf_ready=0
  for _ in $(seq 1 100); do
    if curl -sS -o /dev/null --connect-timeout 1 "http://${SF_LISTEN}/static/hello.txt"; then
      sf_ready=1
      break
    fi
    # Abort early if strace/child already exited.
    if ! kill -0 "$SF_PID" 2>/dev/null; then
      break
    fi
    sleep 0.1
  done
  if [[ "$sf_ready" -eq 1 ]]; then
    curl -sS -o "$STAGE/STRACE/large-under-strace.body" "http://${SF_LISTEN}/static/large.bin" || true
  else
    : >"$STAGE/STRACE/large-under-strace.body"
    cp -a "$STAGE/STRACE/sf.err" "$STAGE/STRACE/sf.err.copy" 2>/dev/null || true
  fi
  kill "$SF_PID" 2>/dev/null || true
  wait "$SF_PID" 2>/dev/null || true
  strace_sha=""
  if [[ -s "$STAGE/STRACE/large-under-strace.body" ]]; then
    strace_sha="$(body_sha "$STAGE/STRACE/large-under-strace.body")"
  fi
  sendfile_count=0
  if [[ -f "$STAGE/STRACE/large-sendfile.strace" ]]; then
    sendfile_count="$(grep -cE 'sendfile(64)?\(' "$STAGE/STRACE/large-sendfile.strace" || true)"
  fi
  if [[ "$sf_ready" -eq 1 && "$strace_sha" == "$LARGE_SHA" && "$sendfile_count" -gt 0 ]]; then
    set_r SENDFILE_REALITY "PASS_count_${sendfile_count}"
  else
    set_r SENDFILE_REALITY "FAIL_ready_${sf_ready}_count_${sendfile_count}_sha_${strace_sha}"
    failures=$((failures + 1))
  fi
else
  set_r SENDFILE_REALITY NOT_MEASURED_STRACE_ABSENT
fi

cp -a "$WORKDIR/dp.out" "$STAGE/RESOURCES/dp.out" 2>/dev/null || true
cp -a "$WORKDIR/dp.err" "$STAGE/RESOURCES/dp.err" 2>/dev/null || true
{
  echo "USES_REAL_DATA=YES"
  echo "USES_SIMULATED_DATA=NO"
  echo "USES_DEMO_DATA=NO"
  echo "USES_MOCKS=NO"
  echo "USES_SYNTHETIC_FIXTURES=YES_LOCAL_DOCROOT_CONTENT_NOT_PRODUCT_RESULT"
  echo "CHANGES_PRODUCT_SEMANTICS=NO"
  echo "ADDS_BENCHMARK_SPECIAL_CASE=NO"
  echo "ADDS_DEMO_SPECIAL_CASE=NO"
  echo "ADDS_PATH_SPECIAL_CASE=NO"
  echo "SKIPS_REAL_COMPONENT=NO"
  echo "CORE_MATRIX_FAILURES=$failures"
} | tee "$STAGE/LEDGER/integrity.txt"

if [[ "$failures" -eq 0 ]]; then
  set_r CORE_MATRIX PASS
  echo "STAGE=$STAGE"
  exit 0
fi

set_r CORE_MATRIX "FAIL_failures_${failures}"
echo "STAGE=$STAGE"
exit 1
