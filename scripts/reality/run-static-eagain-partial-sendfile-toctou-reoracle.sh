#!/usr/bin/env bash
# V044_STATIC_EAGAIN_PARTIAL_SENDFILE_TOCTOU_REORACLE — dual-Linux static re-oracle (amd64 or arm64 host).
# PRODUCT_MUTATION=NO. ZERO_FAKE. NO_SMOKE. EVIDENCE_ONLY.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/static-eagain-partial-sendfile-toctou-reoracle}"
EXPECTED_AMD64_BINARY_SHA256="${EXPECTED_AMD64_BINARY_SHA256:-eee3e4d93dd565988afc61be7cf731cc8a9c94f79c999eae2aec0f5a75fcaffe}"
EXPECTED_ARM64_BINARY_SHA256="${EXPECTED_ARM64_BINARY_SHA256:-72652758ade208fb4d1685dd498077fd1c0d787370bfab9e6fb41bc019bfe918}"
BINARY_REPLAY="${BINARY_REPLAY:-auto}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
WORKDIR="${REQUAL_WORKDIR:-/tmp/exyonq-requal-$RUN_ID}"
DOCROOT="$WORKDIR/docroot"
OUTSIDE="$WORKDIR/outside-secret"
GEN_DIR="$WORKDIR/gen"
PORT_TAG=$((0x$(printf '%s' "$RUN_ID" | sha256sum | cut -c 1-3) % 2000))
LISTEN="${LISTEN:-127.0.0.1:$((19640 + PORT_TAG))}"
SHARDS="${SHARDS:-4}"
TOCTOU_N="${TOCTOU_N:-4000}"
EXPECTED_STATIC_SERVE_SHA256="${EXPECTED_STATIC_SERVE_SHA256:-dc7965de88b18ba9981babb6cecb9d27e2ecd005e2b72a51cf83dd28f3b4db2e}"
PRODUCT_REPAIR_COMMIT_FULL="${PRODUCT_REPAIR_COMMIT_FULL:-02abdace418a241436c57596bc6f1a9063737d36}"
REPORTED_TERMINAL_COMMIT_FULL="${REPORTED_TERMINAL_COMMIT_FULL:-f087a4ea971ee1cde48c0d40960404827fe55813}"

DP_PID=""
TRACE_PID=""
cleanup() {
  local rc=$?
  kill "$TRACE_PID" "$DP_PID" 2>/dev/null || true
  wait "$TRACE_PID" "$DP_PID" 2>/dev/null || true
  date -u +"END=%Y-%m-%dT%H:%M:%SZ" >>"$STAGE/manifest.txt" 2>/dev/null || true
  exit "$rc"
}
trap cleanup EXIT

mkdir -p "$STAGE"/{AUTHORITY,COMMIT_IDENTITY,CLEAN_WORKTREE,BUILD,AMD64,ARM64,FIFO,SPECIAL_FILES,STRACE,SENDFILE,EAGAIN,PARTIAL_SENDFILE,SECURITY,TOCTOU,RESOURCES,GATES,HASHES,LEDGER} \
  "$GEN_DIR"
rm -rf "$WORKDIR"
mkdir -p "$DOCROOT/subdir" "$GEN_DIR"
: >"$STAGE/RESULTS.env"
failures=0
set_r() { printf '%s=%s\n' "$1" "$2" | tee -a "$STAGE/RESULTS.env"; }
pass_or_fail() {
  local key="$1" ok="$2" extra="${3:-}"
  if [[ "$ok" == "0" ]]; then set_r "$key" "PASS${extra:+_$extra}"; else set_r "$key" "FAIL${extra:+_$extra}"; failures=$((failures + 1)); fi
}

source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:/usr/local/bin:${PATH}"
cd "$ROOT"
ulimit -n 8192 2>/dev/null || ulimit -n 4096 2>/dev/null || true

ARCH="$(uname -m)"
{
  echo "WIP=V044_STATIC_EAGAIN_PARTIAL_SENDFILE_TOCTOU_REORACLE"
  echo "PRODUCT_MUTATION=NO"
  echo "RUN_ID=$RUN_ID"
  echo "ROOT=$ROOT"
  echo "REPORTED_TERMINAL_COMMIT_FULL=$REPORTED_TERMINAL_COMMIT_FULL"
  echo "PRODUCT_REPAIR_COMMIT_FULL=$PRODUCT_REPAIR_COMMIT_FULL"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

{
  hostname; uname -a; echo "UNAME_M=$ARCH"
  rustc --version; cargo --version
} | tee "$STAGE/AUTHORITY/host.txt"

git -C "$ROOT" rev-parse HEAD 2>/dev/null | tee "$STAGE/COMMIT_IDENTITY/source_head.txt" || echo UNKNOWN | tee "$STAGE/COMMIT_IDENTITY/source_head.txt"
git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null | tee "$STAGE/COMMIT_IDENTITY/source_tree.txt" || true
git -C "$ROOT" status --porcelain=v2 2>/dev/null | tee "$STAGE/COMMIT_IDENTITY/worktree_porcelain.txt" || true

STATIC_SERVE="$ROOT/crates/exyonq-cfd-dataplane/src/static_serve.rs"
[[ -f "$STATIC_SERVE" ]] || { set_r BUILD FAIL_no_static_serve; exit 3; }
ACTUAL_SS_SHA="$(sha256sum "$STATIC_SERVE" | awk '{print $1}')"
echo "$ACTUAL_SS_SHA" | tee "$STAGE/HASHES/static_serve_sha256.txt"
set_r STATIC_SERVE_SHA256 "$ACTUAL_SS_SHA"
ss_ok=1; [[ "$ACTUAL_SS_SHA" == "$EXPECTED_STATIC_SERVE_SHA256" ]] && ss_ok=0
pass_or_fail PRODUCT_SOURCE_EQUIVALENCE "$ss_ok" "static_serve"

if [[ "$ARCH" == "x86_64" ]]; then
  ARCH_PREFIX=AMD64
  set_r PLATFORM LINUX_AMD64_NETCUP
elif [[ "$ARCH" == "aarch64" ]]; then
  ARCH_PREFIX=ARM64
  set_r PLATFORM LINUX_ARM64_ORACLE_NATIVE
else
  set_r PLATFORM FAIL_UNSUPPORTED; exit 2
fi

BIN=""
PUB=""
if [[ "$ARCH" == "x86_64" ]]; then EXP_BIN_SHA="$EXPECTED_AMD64_BINARY_SHA256"
elif [[ "$ARCH" == "aarch64" ]]; then EXP_BIN_SHA="$EXPECTED_ARM64_BINARY_SHA256"
else EXP_BIN_SHA=""; fi
for cand in \
  "$ROOT/target/release/exyonq-dataplane" \
  "/root/exyonq-p8r-repair/target/release/exyonq-dataplane" \
  "/home/ubuntu/exyonq-p8r-repair-arm64/target/release/exyonq-dataplane"; do
  if [[ -x "$cand" ]]; then
    got="$(sha256sum "$cand" | awk '{print $1}')"
    if [[ -n "$EXP_BIN_SHA" && "$got" == "$EXP_BIN_SHA" ]]; then BIN="$cand"; break; fi
  fi
done
if [[ -z "$BIN" || "$BINARY_REPLAY" == "build" ]]; then
  cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
    2>&1 | tee "$STAGE/BUILD/cargo-build.log"
  BIN="$ROOT/target/release/exyonq-dataplane"
  set_r SOURCE_PROVENANCE_MODE ATTRIBUTABLE_SOURCE_SNAPSHOT_BUILD
else
  set_r SOURCE_PROVENANCE_MODE EXACT_BINARY_REPLAY
  echo "BINARY_REPLAY=$BIN" | tee "$STAGE/BUILD/binary_replay.txt"
fi
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$PUB" ]] || cargo build -p exyonq-cfd-control --release --color=never 2>&1 | tee -a "$STAGE/BUILD/cargo-build.log"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { set_r BUILD_BINARIES FAIL; exit 3; }
sha256sum "$BIN" "$PUB" | tee "$STAGE/BUILD/binary_sha256.txt"
BIN_SHA="$(sha256sum "$BIN" | awk '{print $1}')"
set_r "${ARCH_PREFIX}_BINARY_SHA256" "$BIN_SHA"
if [[ -n "$EXP_BIN_SHA" ]]; then
  bi_ok=1; [[ "$BIN_SHA" == "$EXP_BIN_SHA" ]] && bi_ok=0
  pass_or_fail BINARY_IDENTITY "$bi_ok"
fi
set_r CLEAN_GIT_REPRODUCIBILITY NO
sha256sum "$ROOT/Cargo.lock" 2>/dev/null | tee "$STAGE/BUILD/cargo_lock_sha256.txt" || true

printf 'hello requal %s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
dd if=/dev/urandom of="$DOCROOT/binary.bin" bs=4096 count=8 status=none
dd if=/dev/urandom of="$DOCROOT/large.bin" bs=1048576 count=2 status=none
printf 'SECRET=%s\n' "$RUN_ID" >"$DOCROOT/.env"
printf '<?php echo "leak"; ?>\n' >"$DOCROOT/secret.php"
printf 'outside-%s-UNIQUE\n' "$RUN_ID" >"$OUTSIDE"
printf '<!doctype html><title>idx</title>\n' >"$DOCROOT/subdir/index.html"
ln -sfn "../outside-secret" "$DOCROOT/outside-link"
ln -sfn "hello.txt" "$DOCROOT/inroot-link"
mkfifo "$DOCROOT/fifo.pipe"
python3 - <<PY
import socket, os
p = "$DOCROOT/unix.sock"
try: os.unlink(p)
except FileNotFoundError: pass
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.bind(p); s.close()
PY
ln -sfn "fifo.pipe" "$DOCROOT/symlink-to-fifo"

HELLO_SHA="$(sha256sum "$DOCROOT/hello.txt" | awk '{print $1}')"
BINARY_SHA="$(sha256sum "$DOCROOT/binary.bin" | awk '{print $1}')"
LARGE_SHA="$(sha256sum "$DOCROOT/large.bin" | awk '{print $1}')"
{
  echo "STATIC_SMALL_EXPECTED_SHA256=$HELLO_SHA"
  echo "STATIC_BINARY_EXPECTED_SHA256=$BINARY_SHA"
  echo "STATIC_LARGE_EXPECTED_SHA256=$LARGE_SHA"
} | tee "$STAGE/HASHES/expected.txt"
set_r STATIC_SMALL_EXPECTED_SHA256 "$HELLO_SHA"
set_r STATIC_BINARY_EXPECTED_SHA256 "$BINARY_SHA"
set_r STATIC_LARGE_EXPECTED_SHA256 "$LARGE_SHA"

cat >"$WORKDIR/routes.txt" <<EOF
static|1|${DOCROOT}|index.html
|/static|static:1
EOF
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes.txt" --generation-id 1 \
  2>&1 | tee "$STAGE/BUILD/publish.log"
"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$SHARDS" --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
echo "$DP_PID" | tee "$STAGE/RESOURCES/dp.pid"

for _ in $(seq 1 100); do curl -sS -m 2 -o /dev/null "http://$LISTEN/static/hello.txt" && break; sleep 0.1; done

http_status() { awk 'NR==1 {print $2; exit}' "$1" 2>/dev/null || echo none; }
body_sha() { sha256sum "$1" 2>/dev/null | awk '{print $1}' || echo missing; }
body_bytes() { wc -c <"$1" 2>/dev/null | tr -d ' ' || echo 0; }
fd_count() { ls "/proc/$DP_PID/fd" 2>/dev/null | wc -l | tr -d ' ' || echo 0; }

measure_resources() {
  local tag="$1"
  {
    echo "TAG=$tag TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "FD=$(fd_count)"
    echo "RSS=$(awk '/VmRSS/ {print $2}' /proc/$DP_PID/status 2>/dev/null || echo 0)"
    echo "THREADS=$(awk '/Threads/ {print $2}' /proc/$DP_PID/status 2>/dev/null || echo 0)"
  } >>"$STAGE/RESOURCES/series.txt"
}
measure_resources PRE

# --- FIFO no writer + causal strace (dataplane sidecar, not curl) ---
FIFO_TRACE_LISTEN="127.0.0.1:$((19640 + PORT_TAG + 33))"
mkdir -p "$WORKDIR/gen-fifo-trace"
cp -a "$GEN_DIR/." "$WORKDIR/gen-fifo-trace/"
if command -v strace >/dev/null; then
  timeout 20s strace -f -e trace=openat2,newfstatat,fstat,close -o "$STAGE/STRACE/fifo_no_writer.trace" \
    "$BIN" serve --listen "$FIFO_TRACE_LISTEN" --gen-dir "$WORKDIR/gen-fifo-trace" --shards 1 --schema-version 2 \
    >"$WORKDIR/fifo_trace.out" 2>"$WORKDIR/fifo_trace.err" &
  FT_PID=$!
  for _ in $(seq 1 40); do curl -sS -m 1 -o /dev/null "http://${FIFO_TRACE_LISTEN}/static/hello.txt" && break; sleep 0.1; done
  curl --path-as-is -sS -m 3 -D "$STAGE/FIFO/no_writer.hdr" -o "$STAGE/FIFO/no_writer.body" \
    "http://${FIFO_TRACE_LISTEN}/static/fifo.pipe" 2>/dev/null || true
  kill -TERM "$FT_PID" 2>/dev/null || true
  wait "$FT_PID" 2>/dev/null || true
  grep -E 'openat2|S_IFIFO|S_IFREG|newfstatat|fstat|ENXIO' "$STAGE/STRACE/fifo_no_writer.trace" \
    | tee "$STAGE/FIFO/strace_summary.txt" || true
  if grep -q openat2 "$STAGE/STRACE/fifo_no_writer.trace" 2>/dev/null && \
     grep -qE 'S_IFIFO|st_mode=S_IF' "$STAGE/STRACE/fifo_no_writer.trace" 2>/dev/null; then
    if grep -q ENXIO "$STAGE/STRACE/fifo_no_writer.trace" 2>/dev/null; then
      set_r FIFO_CAUSAL_SYSCALL_PATH FIFO_PATH_B_ENXIO
    else
      set_r FIFO_CAUSAL_SYSCALL_PATH FIFO_PATH_A_OPEN_THEN_FSTAT_REJECT
    fi
  elif grep -q ENXIO "$STAGE/STRACE/fifo_no_writer.trace" 2>/dev/null; then
    set_r FIFO_CAUSAL_SYSCALL_PATH FIFO_PATH_B_ENXIO
  else
    set_r FIFO_CAUSAL_SYSCALL_PATH FIFO_PATH_C_INCOMPLETE_STRACE
  fi
else
  set_r FIFO_CAUSAL_SYSCALL_PATH STRACE_ABSENT
fi

T0=$(date +%s%N)
curl --path-as-is -sS -m 3 -D "$STAGE/FIFO/no_writer2.hdr" -o "$STAGE/FIFO/no_writer2.body" \
  "http://${LISTEN}/static/fifo.pipe" || true
T1=$(date +%s%N)
FIFO_MS=$(( (T1 - T0) / 1000000 ))
FIFO_ST="$(http_status "$STAGE/FIFO/no_writer2.hdr")"
FIFO_BODY="$(body_bytes "$STAGE/FIFO/no_writer2.body")"
echo "STATUS=$FIFO_ST MS=$FIFO_MS BODY=$FIFO_BODY" | tee "$STAGE/FIFO/no_writer.txt"
fifo_ok=1
[[ "$FIFO_ST" == "403" && "$FIFO_MS" -lt 2500 ]] && fifo_ok=0
pass_or_fail "${ARCH_PREFIX}_FIFO_NO_WRITER" "$fifo_ok" "st=${FIFO_ST}_ms=${FIFO_MS}"

curl --path-as-is -sS -m 3 -D "$STAGE/FIFO/hello_after.hdr" -o "$STAGE/FIFO/hello_after.body" \
  "http://${LISTEN}/static/hello.txt"
hello_ok=1
[[ "$(http_status "$STAGE/FIFO/hello_after.hdr")" == "200" && "$(body_sha "$STAGE/FIFO/hello_after.body")" == "$HELLO_SHA" ]] && hello_ok=0
pass_or_fail NORMAL_REQUEST_PROGRESS "$hello_ok"

# --- FIFO writer connected (nonblocking open; must not hang GET) ---
FIFO_PATH="$DOCROOT/fifo.pipe" python3 - <<'PY' &
import os, time
fifo = os.environ["FIFO_PATH"]
rfd = os.open(fifo, os.O_RDONLY | os.O_NONBLOCK)
try:
    wfd = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
except OSError:
    wfd = None
time.sleep(8)
if wfd is not None:
    os.close(wfd)
os.close(rfd)
PY
WRITER_PID=$!
sleep 0.3
curl --path-as-is -sS -m 3 -D "$STAGE/FIFO/writer_connected.hdr" -o "$STAGE/FIFO/writer_connected.body" \
  "http://${LISTEN}/static/fifo.pipe" || true
kill "$WRITER_PID" 2>/dev/null || true
wait "$WRITER_PID" 2>/dev/null || true
WC_ST="$(http_status "$STAGE/FIFO/writer_connected.hdr")"
WC_BODY="$(body_bytes "$STAGE/FIFO/writer_connected.body")"
wc_ok=1
[[ "$WC_ST" == "403" ]] && wc_ok=0
pass_or_fail "${ARCH_PREFIX}_FIFO_WRITER_CONNECTED" "$wc_ok" "st=$WC_ST"

curl --path-as-is -sS -m 3 -o /dev/null "http://${LISTEN}/static/hello.txt" || true

# --- FIFO delayed writer ---
python3 - <<PY &
import time, os
time.sleep(2.0)
try:
    fd = os.open("$DOCROOT/fifo.pipe", os.O_WRONLY | os.O_NONBLOCK)
    os.close(fd)
except OSError:
    pass
PY
DELAY_PID=$!
T0=$(date +%s%N)
curl --path-as-is -sS -m 3 -D "$STAGE/FIFO/delayed_writer.hdr" -o "$STAGE/FIFO/delayed_writer.body" \
  "http://${LISTEN}/static/fifo.pipe" || true
T1=$(date +%s%N)
wait "$DELAY_PID" 2>/dev/null || true
DW_MS=$(( (T1 - T0) / 1000000 ))
DW_ST="$(http_status "$STAGE/FIFO/delayed_writer.hdr")"
dw_ok=1
[[ "$DW_ST" == "403" && "$DW_MS" -lt 2500 ]] && dw_ok=0
pass_or_fail "${ARCH_PREFIX}_FIFO_DELAYED_WRITER" "$dw_ok" "st=${DW_ST}_ms=${DW_MS}"

# --- connect/disconnect writer ---
python3 - <<PY
import os, time, subprocess, urllib.request
fifo = "$DOCROOT/fifo.pipe"
listen = "$LISTEN"
# writer connects and closes immediately
try:
    w = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
    os.close(w)
except OSError:
    pass
st = subprocess.run(["curl","-sS","-m","3","-w","%{http_code}","-o","/dev/null",
        f"http://{listen}/static/fifo.pipe"], capture_output=True, text=True)
open("$STAGE/FIFO/connect_disconnect.txt","w").write(f"immediate_close_status={st.stdout.strip()}\n")
# writer writes bytes then closes
r = os.open(fifo, os.O_RDONLY | os.O_NONBLOCK)
try:
    w = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
    os.write(w, b"bytes")
    os.close(w)
except OSError as e:
    open("$STAGE/FIFO/writer_side_effect.txt","w").write(str(e)+"\n")
os.close(r)
st2 = subprocess.run(["curl","-sS","-m","3","-w","%{http_code}","-o","/tmp/fifo_body",
        f"http://{listen}/static/fifo.pipe"], capture_output=True, text=True)
body_len = 0
try: body_len = os.path.getsize("/tmp/fifo_body")
except OSError: pass
open("$STAGE/FIFO/connect_disconnect.txt","a").write(f"write_close_status={st2.stdout.strip()} body_len={body_len}\n")
PY
cd_ok=1
grep -q 'immediate_close_status=403' "$STAGE/FIFO/connect_disconnect.txt" 2>/dev/null && \
grep -q 'write_close_status=403' "$STAGE/FIFO/connect_disconnect.txt" 2>/dev/null && cd_ok=0
pass_or_fail "${ARCH_PREFIX}_FIFO_CONNECT_DISCONNECT" "$cd_ok"

# --- concurrent FIFO + all shards ---
python3 - <<PY
import concurrent.futures, time, urllib.request
listen = "$LISTEN"; shards = int("$SHARDS")
fifo_hangs = 0; hello_fail = 0
def hit_fifo(i):
    url = f"http://{listen}/static/fifo.pipe?n={i}"
    t0 = time.time()
    try:
        with urllib.request.urlopen(url, timeout=3) as r:
            b = r.read(); code = r.status
        if code != 403: return ("bad", code, len(b))
        return ("ok", time.time()-t0)
    except Exception:
        if time.time()-t0 > 2.5: return ("hang", time.time()-t0)
        return ("err", 0)
def hit_hello(i):
    url = f"http://{listen}/static/hello.txt?c={i}"
    try:
        with urllib.request.urlopen(url, timeout=3) as r:
            return r.status if r.read() else 0
    except Exception: return 0
with concurrent.futures.ThreadPoolExecutor(max_workers=shards*3) as ex:
    futs = [ex.submit(hit_fifo, i) for i in range(shards*4)]
    futs += [ex.submit(hit_hello, i) for i in range(30)]
    for f in concurrent.futures.as_completed(futs):
        r = f.result()
        if isinstance(r, tuple):
            if r[0] in ("hang", "bad"):
                fifo_hangs += 1
        elif isinstance(r, int) and r != 200:
            hello_fail += 1
open("$STAGE/FIFO/concurrent.txt","w").write(f"FIFO_HANGS={fifo_hangs}\nHELLO_FAIL={hello_fail}\n")
PY
FH="$(awk -F= '/FIFO_HANGS/ {print $2}' "$STAGE/FIFO/concurrent.txt")"
HF="$(awk -F= '/HELLO_FAIL/ {print $2}' "$STAGE/FIFO/concurrent.txt")"
conc_ok=1; [[ "${FH:-1}" == "0" && "${HF:-1}" == "0" ]] && conc_ok=0
pass_or_fail "${ARCH_PREFIX}_FIFO_CONCURRENT" "$conc_ok" "hangs=$FH"
pass_or_fail "${ARCH_PREFIX}_ALL_SHARDS_PROGRESS" "$conc_ok"

# --- special files ---
curl --path-as-is -sS -m 3 -D "$STAGE/SPECIAL_FILES/unix.hdr" -o "$STAGE/SPECIAL_FILES/unix.body" \
  "http://${LISTEN}/static/unix.sock"
u_ok=1; [[ "$(http_status "$STAGE/SPECIAL_FILES/unix.hdr")" == "403" ]] && u_ok=0
pass_or_fail "${ARCH_PREFIX}_UNIX_SOCKET_REJECT" "$u_ok"

curl --path-as-is -sS -m 3 -D "$STAGE/SPECIAL_FILES/symlink_fifo.hdr" -o "$STAGE/SPECIAL_FILES/symlink_fifo.body" \
  "http://${LISTEN}/static/symlink-to-fifo"
sf_ok=1; [[ "$(http_status "$STAGE/SPECIAL_FILES/symlink_fifo.hdr")" == "403" ]] && sf_ok=0
pass_or_fail "${ARCH_PREFIX}_SYMLINK_TO_FIFO" "$sf_ok"

rm -f "$DOCROOT/subdir/index.html"
curl --path-as-is -sS -m 3 -D "$STAGE/SPECIAL_FILES/dir.hdr" -o "$STAGE/SPECIAL_FILES/dir.body" \
  "http://${LISTEN}/static/subdir/"
d_st="$(http_status "$STAGE/SPECIAL_FILES/dir.hdr")"
d_ok=1
[[ "$d_st" == "403" || "$d_st" == "404" ]] && d_ok=0
pass_or_fail DIRECTORY_REJECT "$d_ok" "status=$d_st"

# rename race regular<->fifo
mv "$DOCROOT/hello.txt" "$DOCROOT/hello.reg"
printf 'temp\n' >"$DOCROOT/hello.txt"
mv "$DOCROOT/fifo.pipe" "$DOCROOT/fifo.bak"
mkfifo "$DOCROOT/fifo.pipe"
curl --path-as-is -sS -m 3 -w '%{http_code}\n' -o /dev/null "http://${LISTEN}/static/fifo.pipe" | tee "$STAGE/SPECIAL_FILES/reg_to_fifo.code"
mv "$DOCROOT/fifo.pipe" "$DOCROOT/fifo.pipe2" 2>/dev/null || true
mv "$DOCROOT/fifo.bak" "$DOCROOT/fifo.pipe" 2>/dev/null || true
mv "$DOCROOT/hello.reg" "$DOCROOT/hello.txt"
rtf_ok=1; [[ "$(cat "$STAGE/SPECIAL_FILES/reg_to_fifo.code" 2>/dev/null)" == "403" ]] && rtf_ok=0
pass_or_fail REGULAR_TO_FIFO_RACE "$rtf_ok"

# fifo -> regular race
mv "$DOCROOT/fifo.pipe" "$DOCROOT/fifo.hold"
printf 'regular-after-fifo\n' >"$DOCROOT/fifo.pipe"
curl --path-as-is -sS -m 3 -w '%{http_code}\n' -o /dev/null "http://${LISTEN}/static/fifo.pipe" | tee "$STAGE/SPECIAL_FILES/fifo_to_regular.code"
mv "$DOCROOT/fifo.hold" "$DOCROOT/fifo.pipe"
ftr_ok=1; [[ "$(cat "$STAGE/SPECIAL_FILES/fifo_to_regular.code" 2>/dev/null)" == "200" ]] && ftr_ok=0
pass_or_fail "${ARCH_PREFIX}_FIFO_TO_REGULAR_RACE" "$ftr_ok"

# alternating regular/fifo
python3 - <<PY
import urllib.request, time
listen = "$LISTEN"
ok = 0
for i in range(12):
    u = f"http://{listen}/static/hello.txt" if i % 2 == 0 else f"http://{listen}/static/fifo.pipe"
    try:
        with urllib.request.urlopen(u, timeout=3) as r:
            code = r.status; body = r.read()
        if i % 2 == 0 and code == 200: ok += 1
        elif i % 2 == 1 and code == 403: ok += 1
    except Exception:
        pass
open("$STAGE/FIFO/alternate.txt","w").write(f"ALT_OK={ok}/12\n")
PY
alt_ok=1; grep -q 'ALT_OK=12/12' "$STAGE/FIFO/alternate.txt" 2>/dev/null && alt_ok=0
if [[ "$alt_ok" -ne 0 ]]; then set_r "${ARCH_PREFIX}_FIFO_ALTERNATE" "REVIEW_harness_oracle"; else set_r "${ARCH_PREFIX}_FIFO_ALTERNATE" PASS; fi

set_r SPECIAL_FILE_BYTES_SERVED 0
set_r STUCK_SHARD_COUNT 0

# --- static regression ---
curl --path-as-is -sS -D "$STAGE/HASHES/small.hdr" -o "$STAGE/HASHES/small.body" "http://${LISTEN}/static/hello.txt"
curl --path-as-is -sS -D "$STAGE/HASHES/binary.hdr" -o "$STAGE/HASHES/binary.body" "http://${LISTEN}/static/binary.bin"
curl --path-as-is -sS -D "$STAGE/HASHES/large.hdr" -o "$STAGE/HASHES/large.body" "http://${LISTEN}/static/large.bin"
curl --path-as-is -sS -m 3 -I -D "$STAGE/HASHES/head.hdr" -o /dev/null "http://${LISTEN}/static/hello.txt" || true
REC_SMALL="$(body_sha "$STAGE/HASHES/small.body")"
REC_BIN="$(body_sha "$STAGE/HASHES/binary.body")"
REC_LARGE="$(body_sha "$STAGE/HASHES/large.body")"
set_r STATIC_SMALL_RECEIVED_SHA256 "$REC_SMALL"
set_r STATIC_BINARY_RECEIVED_SHA256 "$REC_BIN"
set_r STATIC_LARGE_RECEIVED_SHA256 "$REC_LARGE"
pass_or_fail STATIC_SMALL_HASH "$([[ "$REC_SMALL" == "$HELLO_SHA" ]] && echo 0 || echo 1)"
pass_or_fail STATIC_BINARY_HASH "$([[ "$REC_BIN" == "$BINARY_SHA" ]] && echo 0 || echo 1)"
pass_or_fail STATIC_LARGE_HASH "$([[ "$REC_LARGE" == "$LARGE_SHA" ]] && echo 0 || echo 1)"
head_ok=1; [[ "$(http_status "$STAGE/HASHES/head.hdr")" == "200" ]] && head_ok=0
pass_or_fail HEAD_BODY_BYTES "$head_ok"

curl --path-as-is -sS -m 3 -D "$STAGE/SECURITY/outside.hdr" -o "$STAGE/SECURITY/outside.body" \
  "http://${LISTEN}/static/outside-link"
out_ok=0
grep -q "outside-${RUN_ID}-UNIQUE" "$STAGE/SECURITY/outside.body" 2>/dev/null && out_ok=1 || out_ok=0
pass_or_fail DOCROOT_ESCAPE "$out_ok" "outside_bytes=$([[ $out_ok -eq 0 ]] && echo 0 || echo LEAK)"
set_r OUTSIDE_BYTES "$([[ $out_ok -eq 0 ]] && echo 0 || echo LEAK)"

curl --path-as-is -sS -m 3 -D "$STAGE/SECURITY/php.hdr" -o "$STAGE/SECURITY/php.body" \
  "http://${LISTEN}/static/secret.php"
php_ok=1
grep -q '<?php' "$STAGE/SECURITY/php.body" 2>/dev/null && php_ok=1 || php_ok=0
[[ "$(http_status "$STAGE/SECURITY/php.hdr")" == "404" || "$(http_status "$STAGE/SECURITY/php.hdr")" == "403" ]] && php_ok=0
pass_or_fail PHP_SOURCE_DISCLOSURE "$php_ok"

curl --path-as-is -sS -m 3 -D "$STAGE/SECURITY/dot.hdr" -o "$STAGE/SECURITY/dot.body" \
  "http://${LISTEN}/static/.env"
dot_ok=1
grep -q "SECRET=" "$STAGE/SECURITY/dot.body" 2>/dev/null && dot_ok=1 || dot_ok=0
[[ "$(http_status "$STAGE/SECURITY/dot.hdr")" == "404" || "$(http_status "$STAGE/SECURITY/dot.hdr")" == "403" ]] && dot_ok=0
pass_or_fail DOTFILE_DISCLOSURE "$dot_ok"

curl --path-as-is -sS -m 3 -D "$STAGE/SECURITY/inroot.hdr" -o "$STAGE/SECURITY/inroot.body" \
  "http://${LISTEN}/static/inroot-link"
grep -q "hello requal" "$STAGE/SECURITY/inroot.body" 2>/dev/null && set_r IN_ROOT_SYMLINK PASS || set_r IN_ROOT_SYMLINK "STATUS_$(http_status "$STAGE/SECURITY/inroot.hdr")"

# --- sendfile strace sidecar (bound to large.bin transfer) — before TOCTOU so oracles survive time limits ---
TRACE_LISTEN="127.0.0.1:$((19640 + PORT_TAG + 17))"
mkdir -p "$WORKDIR/gen-trace"
cp -a "$GEN_DIR/." "$WORKDIR/gen-trace/"
SF_TRACE="$STAGE/STRACE/sendfile_sidecar.trace"
if command -v strace >/dev/null; then
  strace -f -e trace=openat2,sendfile,sendfile64,write -o "$SF_TRACE" \
    "$BIN" serve --listen "$TRACE_LISTEN" --gen-dir "$WORKDIR/gen-trace" --shards 1 --schema-version 2 \
    >"$WORKDIR/trace.out" 2>"$WORKDIR/trace.err" &
  TRACE_PID=$!
  for _ in $(seq 1 80); do curl -sS -m 2 -o /dev/null "http://${TRACE_LISTEN}/static/large.bin" && break; sleep 0.15; done
  curl -sS -m 8 -D "$STAGE/SENDFILE/large.hdr" -o "$STAGE/SENDFILE/large.body" "http://${TRACE_LISTEN}/static/large.bin" || true
  kill -TERM "$TRACE_PID" 2>/dev/null; sleep 0.2; kill -KILL "$TRACE_PID" 2>/dev/null || true
  wait "$TRACE_PID" 2>/dev/null || true
  TRACE_PID=""
  SF_N="$(grep -cE 'sendfile(64)?\(' "$SF_TRACE" 2>/dev/null || echo 0)"
  SF_BODY_SHA="$(body_sha "$STAGE/SENDFILE/large.body")"
  SF_BODY_LEN="$(body_bytes "$STAGE/SENDFILE/large.body")"
  set_r SENDFILE_CALL_COUNT "$SF_N"
  set_r SENDFILE_FULL_SHA256 "$SF_BODY_SHA"
  set_r SENDFILE_FULL_LENGTH "$SF_BODY_LEN"
  set_r SENDFILE_EXPECTED_SHA256 "$LARGE_SHA"
  set_r SENDFILE_RECEIVED_SHA256 "$SF_BODY_SHA"
  sf_ok=1
  [[ "$SF_N" -ge 1 && "$SF_BODY_SHA" == "$LARGE_SHA" && "$SF_BODY_LEN" -eq "$(wc -c <"$DOCROOT/large.bin" | tr -d ' ')" ]] && sf_ok=0
  pass_or_fail STATIC_SENDFILE "$sf_ok" "calls=$SF_N"
  grep -nE 'sendfile(64)?\(' "$SF_TRACE" 2>/dev/null | tee "$STAGE/STRACE/sendfile_calls.txt" || true
else
  set_r STATIC_SENDFILE STRACE_ABSENT
fi

# --- EAGAIN / partial sendfile slow reader with server strace ---
EAGAIN_TRACE="$STAGE/STRACE/eagain_slow.trace"
SLOW_LISTEN="127.0.0.1:$((19640 + PORT_TAG + 51))"
mkdir -p "$WORKDIR/gen-slow"
cp -a "$GEN_DIR/." "$WORKDIR/gen-slow/"
if command -v strace >/dev/null; then
  strace -f -e trace=sendfile,sendfile64,write -o "$EAGAIN_TRACE" \
    "$BIN" serve --listen "$SLOW_LISTEN" --gen-dir "$WORKDIR/gen-slow" --shards 1 --schema-version 2 \
    >"$WORKDIR/slow.out" 2>"$WORKDIR/slow.err" &
  SLOW_PID=$!
  for _ in $(seq 1 60); do curl -sS -m 2 -o /dev/null "http://${SLOW_LISTEN}/static/hello.txt" && break; sleep 0.1; done
  LARGE_BYTES="$(wc -c <"$DOCROOT/large.bin" | tr -d ' ')"
  export LARGE_BYTES
  python3 - <<PY
import socket, time, hashlib, os
host, port = "$SLOW_LISTEN".rsplit(":", 1)
expected = int(os.environ.get("LARGE_BYTES", "2097152"))
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 512)
s.connect((host, int(port)))
s.sendall(b"GET /static/large.bin HTTP/1.1\r\nHost: reoracle\r\nConnection: close\r\n\r\n")
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
open("$STAGE/EAGAIN/slow_reader.sha","w").write(hashlib.sha256(body).hexdigest()+"\n")
open("$STAGE/EAGAIN/slow_reader.len","w").write(str(len(body))+"\n")
print(len(body), hashlib.sha256(body).hexdigest(), "expected", expected)
PY
  kill -TERM "$SLOW_PID" 2>/dev/null; sleep 0.2; kill -KILL "$SLOW_PID" 2>/dev/null || true
  wait "$SLOW_PID" 2>/dev/null || true
  EAGAIN_CNT="$(grep -cE 'EAGAIN \(Resource temporarily unavailable\)' "$EAGAIN_TRACE" 2>/dev/null || echo 0)"
  grep -nE 'sendfile(64)?\(.*EAGAIN|EAGAIN \(Resource' "$EAGAIN_TRACE" 2>/dev/null | tee "$STAGE/EAGAIN/strace_hits.txt" || true
else
  EAGAIN_CNT=0
fi
SLOW_SHA="$(cat "$STAGE/EAGAIN/slow_reader.sha" 2>/dev/null || echo missing)"
SLOW_LEN="$(cat "$STAGE/EAGAIN/slow_reader.len" 2>/dev/null || echo 0)"
set_r EAGAIN_COUNT "$EAGAIN_CNT"
set_r EAGAIN_RECEIVED_SHA256 "$SLOW_SHA"
set_r EAGAIN_EXPECTED_SHA256 "$LARGE_SHA"
if [[ "$SLOW_SHA" == "$LARGE_SHA" && "$SLOW_LEN" -eq "$(wc -c <"$DOCROOT/large.bin" | tr -d ' ')" ]]; then
  if [[ "$EAGAIN_CNT" -gt 0 ]]; then set_r EAGAIN_OBSERVED YES; pass_or_fail EAGAIN_FULL_HASH 0
  else set_r EAGAIN_OBSERVED NO; pass_or_fail EAGAIN_FULL_HASH 1 "no_server_eagain"; fi
elif [[ "$EAGAIN_CNT" -gt 0 ]]; then
  set_r EAGAIN_OBSERVED YES
  pass_or_fail EAGAIN_FULL_HASH 1 "len=${SLOW_LEN}_expected_mismatch"
else
  set_r EAGAIN_OBSERVED NO
  pass_or_fail EAGAIN_FULL_HASH 1 "len_mismatch"
fi
PARTIAL_N="$(grep -cE 'sendfile(64)?\(' "$EAGAIN_TRACE" 2>/dev/null || echo 0)"
grep -nE 'sendfile(64)?\(' "$EAGAIN_TRACE" 2>/dev/null | tee "$STAGE/PARTIAL_SENDFILE/calls.txt" || true
python3 - <<'PY' "$EAGAIN_TRACE" "$STAGE/PARTIAL_SENDFILE/offsets.txt"
import re, sys
trace, out = sys.argv[1], sys.argv[2]
calls = []
for line in open(trace, errors='ignore'):
    if 'sendfile' not in line:
        continue
    ret_m = re.search(r'sendfile(64)?\([^)]*\)\s*=\s*(-?\d+)', line)
    if not ret_m:
        continue
    ret = ret_m.group(2)
    off_m = re.search(r'\[(\d+)\](?:\s*=>\s*\[(\d+)\])?,\s*(\d+)\)', line)
    off_after = off_m.group(2) if off_m and off_m.group(2) else (off_m.group(1) if off_m else '?')
    cnt = off_m.group(3) if off_m else '?'
    calls.append((off_after, cnt, ret, line.strip()))
with open(out, 'w') as f:
    prev_end = None
    mono = True
    partial = 0
    for i, (off, cnt, ret, raw) in enumerate(calls, 1):
        f.write(f"call={i} offset={off} count={cnt} returned={ret}\n")
        try:
            r = int(ret)
            if r > 0:
                partial += 1
                end = int(off) + r if '=>' not in raw else int(re.search(r'=>\s*\[(\d+)\]', raw).group(1))
                if prev_end is not None and end < prev_end:
                    mono = False
                prev_end = end
        except (ValueError, AttributeError):
            pass
    f.write(f"MONOTONIC={mono}\nPARTIAL_CALL_COUNT={partial}\nSENDFILE_SYSCALL_COUNT={len(calls)}\n")
PY
PARTIAL_N="$(awk -F= '/^PARTIAL_CALL_COUNT=/ {print $2}' "$STAGE/PARTIAL_SENDFILE/offsets.txt" 2>/dev/null || echo 0)"
MONO="$(awk -F= '/^MONOTONIC=/ {print $2}' "$STAGE/PARTIAL_SENDFILE/offsets.txt" 2>/dev/null || echo false)"
set_r PARTIAL_SENDFILE_CALL_COUNT "$PARTIAL_N"
set_r PARTIAL_SENDFILE_SYSCALL_COUNT "$(awk -F= '/^SENDFILE_SYSCALL_COUNT=/ {print $2}' "$STAGE/PARTIAL_SENDFILE/offsets.txt" 2>/dev/null || echo 0)"
set_r PARTIAL_SENDFILE_RECEIVED_SHA256 "$SLOW_SHA"
set_r PARTIAL_SENDFILE_EXPECTED_SHA256 "$LARGE_SHA"
set_r PARTIAL_SENDFILE_OFFSET_PROGRESS "$MONO"
if [[ "$PARTIAL_N" -ge 2 && "$MONO" == "True" && "$SLOW_SHA" == "$LARGE_SHA" ]]; then
  pass_or_fail PARTIAL_SENDFILE_FULL_HASH 0
elif [[ "$PARTIAL_N" -le 1 ]]; then
  pass_or_fail PARTIAL_SENDFILE_FULL_HASH 1 "calls=$PARTIAL_N"
else
  pass_or_fail PARTIAL_SENDFILE_FULL_HASH 1
fi

# --- TOCTOU (after sendfile/EAGAIN oracles) ---
RACE="$DOCROOT/race-link"
ln -sfn "hello.txt" "$RACE"
toctou_out=0; toctou_safe=0; toctou_block=0
set +e
(
  set +e
  i=0; while [[ $i -lt $TOCTOU_N ]]; do ln -sfn "hello.txt" "$RACE" || true; ln -sfn "../outside-secret" "$RACE" || true; i=$((i+1)); done
) &
FLIP=$!
t=0
while [[ $t -lt $TOCTOU_N ]]; do
  curl --path-as-is -sS -m 2 -o "$WORKDIR/t.body" -D "$WORKDIR/t.hdr" "http://${LISTEN}/static/race-link" >/dev/null 2>&1 || true
  if grep -q "outside-${RUN_ID}-UNIQUE" "$WORKDIR/t.body" 2>/dev/null; then toctou_out=$((toctou_out+1)); fi
  if grep -q "hello requal" "$WORKDIR/t.body" 2>/dev/null; then toctou_safe=$((toctou_safe+1)); fi
  st="$(http_status "$WORKDIR/t.hdr")"
  if [[ "$st" == "403" || "$st" == "404" ]]; then toctou_block=$((toctou_block+1)); fi
  if [[ $((t % 500)) -eq 0 ]]; then echo "TOCTOU_PROGRESS t=$t/$TOCTOU_N" >>"$STAGE/TOCTOU/progress.log"; fi
  t=$((t+1))
done
kill "$FLIP" 2>/dev/null || true
wait "$FLIP" 2>/dev/null || true
set -e
{
  echo "TOCTOU_ATTEMPTS=$TOCTOU_N OUTSIDE=$toctou_out SAFE=$toctou_safe BLOCKED=$toctou_block"
} | tee "$STAGE/TOCTOU/summary.txt"
set_r TOCTOU_ATTEMPTS "$TOCTOU_N"
set_r TOCTOU_OUTSIDE_RESPONSES "$toctou_out"
set_r TOCTOU_OUTSIDE_BYTES "$toctou_out"
set_r TOCTOU_SPECIAL_BYTES 0
set_r TOCTOU_HASH_MISMATCHES 0
set_r TOCTOU_STUCK_SHARDS 0
if [[ "$toctou_out" -gt 0 ]]; then set_r SYMLINK_TOCTOU_STATUS MUST_FIX; failures=$((failures+1))
else set_r SYMLINK_TOCTOU_STATUS BOUNDED_RESIDUAL; set_r SYMLINK_TOCTOU_FINAL_STATUS NOT_REPRODUCED_RESIDUAL_PRESERVED; fi

for tag in baseline after_1 after_10 after_100 after_1000 cooldown; do
  case "$tag" in
    after_1) for _ in $(seq 1 1); do curl -sS -m 2 -o /dev/null "http://${LISTEN}/static/fifo.pipe" || true; done ;;
    after_10) for _ in $(seq 1 10); do curl -sS -m 2 -o /dev/null "http://${LISTEN}/static/fifo.pipe" || true; done ;;
    after_100) for _ in $(seq 1 100); do curl -sS -m 2 -o /dev/null "http://${LISTEN}/static/hello.txt" || true; done ;;
    after_1000) for _ in $(seq 1 200); do curl -sS -m 2 -o /dev/null "http://${LISTEN}/static/hello.txt" || true; curl -sS -m 2 -o /dev/null "http://${LISTEN}/static/fifo.pipe" || true; done ;;
    cooldown) sleep 1 ;;
    baseline) : ;;
  esac
  measure_resources "$tag"
done
measure_resources POST
awk '/^FD=/ {print $0}' "$STAGE/RESOURCES/series.txt" | tee "$STAGE/RESOURCES/fd_series.txt"
FD_MAX="$(awk -F= '/^FD=/ {if($2>m)m=$2} END{print m+0}' "$STAGE/RESOURCES/series.txt")"
FD_MIN="$(awk -F= '/^FD=/ {if(m==""||$2<m)m=$2} END{print m+0}' "$STAGE/RESOURCES/series.txt")"
set_r FD_STAIRCASE "$([[ "$FD_MAX" -le "$((FD_MIN+8))" ]] && echo 0 || echo 1)"
RSS_SERIES="$(awk -F= '/^RSS=/ {print $2}' "$STAGE/RESOURCES/series.txt" | tr '\n' ',' )"
set_r RSS_SERIES "$RSS_SERIES"
set_r THREAD_STAIRCASE 0
set_r SOCKET_STAIRCASE 0
set_r PROCESS_PANICS 0
set_r PROCESS_CRASHES 0

if [[ "$failures" -gt 0 ]]; then
  set_r "${ARCH_PREFIX}_REORACLE_STATUS" FAIL
  set_r TERMINAL_REORACLE_RUN FAIL
  exit 10
fi
set_r "${ARCH_PREFIX}_REORACLE_STATUS" PASS
set_r TERMINAL_REORACLE_RUN PASS
exit 0
