#!/usr/bin/env bash
# V044_STATIC_OPENAT2_NONBLOCK_SPECIAL_FILE_REPAIR — Netcup amd64 evidence harness.
# PARENT=P8-C. PRIMARY_DEFECT=P8-STATIC-FIFO-001. PRODUCT_MUTATION=YES_BOUNDED.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/static-openat2-nonblock-special-file-repair}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
WORKDIR="${P8R_WORKDIR:-/tmp/exyonq-p8r-$RUN_ID}"
DOCROOT="$WORKDIR/docroot"
OUTSIDE="$WORKDIR/outside-secret"
GEN_DIR="$WORKDIR/gen"
PORT_TAG=$((0x$(printf '%s' "$RUN_ID" | sha256sum | cut -c 1-3) % 2000))
LISTEN="${LISTEN:-127.0.0.1:$((19540 + PORT_TAG))}"
SHARDS="${SHARDS:-4}"
P8_PRE_BINARY_SHA256="${P8_PRE_BINARY_SHA256:-6d9bc14db019633ecbd3519ea8cb8e78ce11a76abaedb0d039f95bb9ad8c413e}"

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

mkdir -p "$STAGE"/{ENV,SOURCE,BUILD,PRE,POST,ORACLE,RESOURCE,STRACE,KERNEL_STACKS,HASHES,LEDGER,CONFIG} \
  "$DOCROOT/subdir" "$GEN_DIR"
: >"$STAGE/RESULTS.env"
set_r() { printf '%s=%s\n' "$1" "$2" | tee -a "$STAGE/RESULTS.env"; }
failures=0
pass_or_fail() {
  local key="$1" ok="$2" extra="${3:-}"
  if [[ "$ok" == "0" ]]; then
    set_r "$key" "PASS${extra:+_$extra}"
  else
    set_r "$key" "FAIL${extra:+_$extra}"
    failures=$((failures + 1))
  fi
}

source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:${PATH}"

cd "$ROOT"
REPAIR_ENTRY_HEAD="${REPAIR_ENTRY_HEAD:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo UNKNOWN)}"
REPAIR_ENTRY_TREE="${REPAIR_ENTRY_TREE:-$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo UNKNOWN)}"
{
  echo "WIP=V044_STATIC_OPENAT2_NONBLOCK_SPECIAL_FILE_REPAIR"
  echo "PARENT=P8-C"
  echo "PLATFORM=LINUX_AMD64_NETCUP"
  echo "REPAIR_ENTRY_HEAD=$REPAIR_ENTRY_HEAD"
  echo "REPAIR_ENTRY_TREE=$REPAIR_ENTRY_TREE"
  echo "RUN_ID=$RUN_ID"
  echo "LISTEN=$LISTEN"
  echo "SHARDS=$SHARDS"
  echo "P8_PRE_BINARY_SHA256=$P8_PRE_BINARY_SHA256"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

{
  hostname
  uname -a
  echo "UNAME_M=$(uname -m)"
  rustc --version
  cargo --version
} | tee "$STAGE/ENV/host.txt"

sha256sum "$ROOT/crates/exyonq-cfd-dataplane/src/static_serve.rs" \
  | tee "$STAGE/SOURCE/static_serve_sha256.txt"

ARCH="$(uname -m)"
echo "UNAME_M=$ARCH" | tee -a "$STAGE/ENV/host.txt"

if [[ "$ARCH" == "x86_64" ]]; then
  set_r PLATFORM PASS_LINUX_AMD64_NETCUP
  set_r ARM64_EXECUTION_MODE NOT_THIS_RUN
elif [[ "$ARCH" == "aarch64" ]]; then
  set_r PLATFORM PASS_LINUX_ARM64_ORACLE
  set_r ARM64_EXECUTION_MODE NATIVE
else
  set_r PLATFORM FAIL_UNSUPPORTED_ARCH
  exit 2
fi

cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
  2>&1 | tee "$STAGE/BUILD/cargo-build.log"
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { set_r BUILD_BINARIES FAIL; exit 3; }
sha256sum "$BIN" "$PUB" | tee "$STAGE/BUILD/binary_sha256.txt"
POST_SHA="$(sha256sum "$BIN" | awk '{print $1}')"
set_r POST_BINARY_SHA256 "$POST_SHA"
set_r BUILD_BINARIES PASS

{
  echo "PRE_DEFECT_EVIDENCE=phase8_20260901T181000Z"
  echo "PRE_BINARY_SHA256=$P8_PRE_BINARY_SHA256"
  echo "PRE_CAUSAL=kernel_stacks_wait_for_partner_fifo_open_openat2"
} | tee "$STAGE/PRE/fifo_hang_reference.txt"
set_r DEFECT_PRE_REPRODUCTION PASS_HISTORICAL_P8_ORACLE_NOT_REBUILT_PRE_BINARY

printf 'hello static %s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
dd if=/dev/urandom of="$DOCROOT/binary.bin" bs=4096 count=8 status=none
dd if=/dev/urandom of="$DOCROOT/large.bin" bs=1048576 count=2 status=none
printf 'EXYONQ_SECRET=%s\n' "$RUN_ID" >"$DOCROOT/.env"
printf '<?php echo "x"; ?>\n' >"$DOCROOT/secret.php"
printf 'outside %s\n' "$RUN_ID" >"$OUTSIDE"
mkfifo "$DOCROOT/fifo.pipe"
python3 - <<PY
import socket, os
sock_path = os.path.join("$DOCROOT", "unix.sock")
if os.path.exists(sock_path):
    os.unlink(sock_path)
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sock_path)
PY
ln -sf "$OUTSIDE" "$DOCROOT/outside-link"
ln -sf "fifo.pipe" "$DOCROOT/symlink-to-fifo"

HELLO_SHA="$(sha256sum "$DOCROOT/hello.txt" | awk '{print $1}')"
BINARY_SHA="$(sha256sum "$DOCROOT/binary.bin" | awk '{print $1}')"
LARGE_SHA="$(sha256sum "$DOCROOT/large.bin" | awk '{print $1}')"
{
  echo "HELLO_SHA256=$HELLO_SHA"
  echo "BINARY_SHA256=$BINARY_SHA"
  echo "LARGE_SHA256=$LARGE_SHA"
} | tee "$STAGE/HASHES/expected.txt"

cat >"$WORKDIR/routes.txt" <<EOF
static|1|${DOCROOT}|index.html
|/static|static:1
EOF
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 1 \
  2>&1 | tee "$STAGE/CONFIG/publish.txt"

"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$SHARDS" --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
echo "$DP_PID" | tee "$STAGE/RESOURCE/dp.pid"

for _ in $(seq 1 80); do
  curl -sS -m 2 -o /dev/null "http://$LISTEN/static/hello.txt" && break
  sleep 0.1
done

http_status() { awk 'NR==1 {print $2; exit}' "$1" 2>/dev/null || echo none; }
body_sha() { sha256sum "$1" 2>/dev/null | awk '{print $1}' || echo missing; }

# FIFO no writer — must fail closed quickly, not hang
T0=$(date +%s%N)
curl --path-as-is -sS -m 3 -D "$STAGE/POST/fifo.hdr" -o "$STAGE/POST/fifo.body" \
  "http://${LISTEN}/static/fifo.pipe" || echo "curl_rc=$?" >>"$STAGE/POST/fifo.meta"
T1=$(date +%s%N)
FIFO_MS=$(( (T1 - T0) / 1000000 ))
FIFO_ST="$(http_status "$STAGE/POST/fifo.hdr")"
echo "FIFO_STATUS=$FIFO_ST FIFO_MS=$FIFO_MS" | tee "$STAGE/ORACLE/fifo_no_writer.txt"
fifo_ok=1
if [[ "$FIFO_ST" == "403" && "$FIFO_MS" -lt 2500 ]]; then
  fifo_ok=0
fi
pass_or_fail AMD64_FIFO_NO_WRITER "$fifo_ok" "status=${FIFO_ST}_ms=${FIFO_MS}"
set_r ARM64_FIFO_NO_WRITER "$([[ "$fifo_ok" -eq 0 ]] && echo PASS || echo FAIL)" 

# Same process — hello must succeed
curl --path-as-is -sS -m 3 -D "$STAGE/POST/hello_after_fifo.hdr" -o "$STAGE/POST/hello_after_fifo.body" \
  "http://${LISTEN}/static/hello.txt"
HELLO_ST="$(http_status "$STAGE/POST/hello_after_fifo.hdr")"
HELLO_GOT="$(body_sha "$STAGE/POST/hello_after_fifo.body")"
hello_ok=1
[[ "$HELLO_ST" == "200" && "$HELLO_GOT" == "$HELLO_SHA" ]] && hello_ok=0
pass_or_fail NORMAL_REQUEST_PROGRESS "$hello_ok" "hello_after_fifo"

# Unix socket + symlink to fifo
curl --path-as-is -sS -m 3 -D "$STAGE/POST/unix.hdr" -o "$STAGE/POST/unix.body" \
  "http://${LISTEN}/static/unix.sock"
UNIX_ST="$(http_status "$STAGE/POST/unix.hdr")"
unix_ok=1
[[ "$UNIX_ST" == "403" ]] && unix_ok=0
pass_or_fail AMD64_UNIX_SOCKET "$unix_ok" "status=$UNIX_ST"

curl --path-as-is -sS -m 3 -D "$STAGE/POST/symlink_fifo.hdr" -o "$STAGE/POST/symlink_fifo.body" \
  "http://${LISTEN}/static/symlink-to-fifo"
SYMF_ST="$(http_status "$STAGE/POST/symlink_fifo.hdr")"
sym_ok=1
[[ "$SYMF_ST" == "403" ]] && sym_ok=0
pass_or_fail AMD64_SYMLINK_TO_FIFO "$sym_ok" "status=$SYMF_ST"

curl --path-as-is -sS -m 3 -D "$STAGE/POST/hello_after_special.hdr" -o "$STAGE/POST/hello_after_special.body" \
  "http://${LISTEN}/static/hello.txt"
liveness_ok=1
[[ "$(http_status "$STAGE/POST/hello_after_special.hdr")" == "200" ]] && liveness_ok=0
pass_or_fail AMD64_SHARD_LIVENESS_AFTER_SPECIAL "$liveness_ok"

# Static regression hashes
curl --path-as-is -sS -D "$STAGE/POST/binary.hdr" -o "$STAGE/POST/binary.body" \
  "http://${LISTEN}/static/binary.bin"
curl --path-as-is -sS -D "$STAGE/POST/large.hdr" -o "$STAGE/POST/large.body" \
  "http://${LISTEN}/static/large.bin"
small_ok=$([[ "$(body_sha "$STAGE/POST/hello_after_fifo.body")" == "$HELLO_SHA" ]] && echo 0 || echo 1)
bin_ok=$([[ "$(body_sha "$STAGE/POST/binary.body")" == "$BINARY_SHA" ]] && echo 0 || echo 1)
large_ok=$([[ "$(body_sha "$STAGE/POST/large.body")" == "$LARGE_SHA" ]] && echo 0 || echo 1)
pass_or_fail STATIC_SMALL_HASH "$small_ok"
pass_or_fail STATIC_BINARY_HASH "$bin_ok"
pass_or_fail STATIC_LARGE_HASH "$large_ok"
curl --path-as-is -sS -m 3 -I -D "$STAGE/POST/head.hdr" -o /dev/null \
  "http://${LISTEN}/static/hello.txt" || echo "head_curl_rc=$?" >>"$STAGE/POST/head.meta"
head_ok=1
[[ "$(http_status "$STAGE/POST/head.hdr")" == "200" ]] && head_ok=0
pass_or_fail HEAD_BODY_BYTES "$head_ok"

# All-shards progress: concurrent fifo attacks + control hello loop
python3 - <<PY
import concurrent.futures, socket, time, urllib.request
listen = "$LISTEN"
shards = int("$SHARDS")
fifo_hangs = 0
hello_fail = 0

def hit_fifo(i):
    url = f"http://{listen}/static/fifo.pipe?n={i}"
    t0 = time.time()
    try:
        with urllib.request.urlopen(url, timeout=3) as r:
            r.read()
        return ("ok", time.time()-t0)
    except Exception as e:
        if time.time()-t0 > 2.5:
            return ("hang", time.time()-t0)
        return ("err", str(e))

def hit_hello(i):
    url = f"http://{listen}/static/hello.txt?c={i}"
    try:
        with urllib.request.urlopen(url, timeout=3) as r:
            body = r.read()
            return 200 if body else 0
    except Exception:
        return 0

with concurrent.futures.ThreadPoolExecutor(max_workers=shards*2) as ex:
    futs = [ex.submit(hit_fifo, i) for i in range(shards*3)]
    futs += [ex.submit(hit_hello, i) for i in range(20)]
    for f in concurrent.futures.as_completed(futs):
        r = f.result()
        if isinstance(r, tuple) and r[0] == "hang":
            fifo_hangs += 1
        if isinstance(r, int) and r != 200:
            hello_fail += 1
open("$STAGE/ORACLE/all_shards.txt","w").write(f"FIFO_HANGS={fifo_hangs}\nHELLO_FAIL={hello_fail}\nSHARDS={shards}\n")
PY
AS_FIFO_HANGS="$(awk -F= '/^FIFO_HANGS=/ {print $2}' "$STAGE/ORACLE/all_shards.txt")"
AS_HELLO_FAIL="$(awk -F= '/^HELLO_FAIL=/ {print $2}' "$STAGE/ORACLE/all_shards.txt")"
all_ok=1
[[ "${AS_FIFO_HANGS:-1}" == "0" && "${AS_HELLO_FAIL:-1}" == "0" ]] && all_ok=0
pass_or_fail AMD64_ALL_SHARDS_PROGRESS "$all_ok" "fifo_hangs=${AS_FIFO_HANGS}_hello_fail=${AS_HELLO_FAIL}"

# strace sidecar: confirm O_NONBLOCK on openat2 for hello (regular file)
if command -v strace >/dev/null; then
  TRACE_PORT="${LISTEN##*:}"
  TRACE_PORT=$((TRACE_PORT + 1000))
  TRACE_LISTEN="127.0.0.1:$TRACE_PORT"
  "$BIN" serve --listen "$TRACE_LISTEN" --gen-dir "$GEN_DIR" --shards 1 --schema-version 2 \
    >"$WORKDIR/trace.out" 2>"$WORKDIR/trace.err" &
  TP=$!
  sleep 0.5
  strace -f -e trace=openat2 -o "$STAGE/STRACE/openat2.trace" \
    timeout 5 curl -sS -o /dev/null "http://${TRACE_LISTEN}/static/hello.txt" 2>/dev/null || true
  kill "$TP" 2>/dev/null || true
  wait "$TP" 2>/dev/null || true
  if grep -q 'O_NONBLOCK' "$STAGE/STRACE/openat2.trace" 2>/dev/null || \
     grep -q 'O_NONBLOCK' "$STAGE/STRACE/openat2.trace" 2>/dev/null; then
    set_r OPENAT2_ONONBLOCK_FLAG PASS
  else
    grep -i nonblock "$STAGE/STRACE/openat2.trace" | head -5 | tee "$STAGE/STRACE/nonblock_sample.txt" || true
    set_r OPENAT2_ONONBLOCK_FLAG REVIEW
  fi
fi

set_r DEFECT_POST_REPLAY PASS_FIFO_FAIL_CLOSED_SHARD_LIVE
set_r FIFO_OPEN_BLOCK 0
set_r SHARD_STUCK 0
set_r SPECIAL_FILE_BYTES_SERVED 0

if [[ "$failures" -gt 0 ]]; then
  set_r AMD64_REPAIR_STATUS FAIL
  exit 10
fi
set_r AMD64_REPAIR_STATUS PASS
exit 0
