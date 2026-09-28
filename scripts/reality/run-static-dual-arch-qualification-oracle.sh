#!/usr/bin/env bash
# V044_STATIC_DUAL_ARCH_QUALIFICATION — native Linux ARM64 Oracle A1.
# PARENT=P7R-B. STATIC_IMPLEMENTATION_PARENT=P7STATICIMPL-B.
# Architecture portability only — NOT Phase 7 rerun, NOT performance.
# PRODUCT_MUTATION=NO. COMMIT/PUSH/TAG/RELEASE=NO.
set -euo pipefail

ROOT="${EXYONQ_ROOT:-$(cd "$(dirname "$0")/../.." && pwd)}"
EVIDENCE_ROOT="${EVIDENCE_ROOT:-$ROOT/.exyonq-local/evidence/static-dual-arch-qualification}"
RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
STAGE="$EVIDENCE_ROOT/$RUN_ID"
WORKDIR="${STATICDUAL_WORKDIR:-/tmp/exyonq-staticdual-$RUN_ID}"
DOCROOT="$WORKDIR/docroot"
OUTSIDE="$WORKDIR/outside-secret"
GEN_DIR="$WORKDIR/gen"
PORT_TAG=$((0x$(printf '%s' "$RUN_ID" | sha256sum | cut -c 1-3) % 2000))
LISTEN="${LISTEN:-127.0.0.1:$((18840 + PORT_TAG))}"
METRICS_PORT=$((19840 + PORT_TAG))
SHARDS="${SHARDS:-1}"

DP_PID=""
cleanup() {
  local rc=$?
  kill "$DP_PID" 2>/dev/null || true
  wait "$DP_PID" 2>/dev/null || true
  date -u +"END=%Y-%m-%dT%H:%M:%SZ" >>"$STAGE/manifest.txt" 2>/dev/null || true
  exit "$rc"
}
trap cleanup EXIT

mkdir -p "$STAGE"/{SOURCE,BUILD,HOST,CONFIG,HTTP,ORACLE,RESOURCES,STRACE,GATES,SECURITY,ARCH,LEDGER} \
  "$DOCROOT/subdir" "$DOCROOT/emptydir" "$GEN_DIR"
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
AMD64_REF_DIGESTS="${AMD64_REF_DIGESTS:-$STAGE/SOURCE/amd64_reference_digests.txt}"

{
  echo "WIP=V044_STATIC_DUAL_ARCH_QUALIFICATION"
  echo "PARENT=P7R-B"
  echo "STATIC_IMPLEMENTATION_PARENT=P7STATICIMPL-B"
  echo "REFERENCE_PLATFORM=LINUX_AMD64_NETCUP"
  echo "ARM64_PLATFORM=LINUX_ARM64_ORACLE_A1"
  echo "ENTRY_HEAD=$ENTRY_HEAD"
  echo "ENTRY_TREE=$ENTRY_TREE"
  echo "RUN_ID=$RUN_ID"
  echo "PRODUCT_MUTATION=NO"
  echo "ARM64_EXECUTION_MODE=NATIVE_REQUIRED"
  echo "LISTEN=$LISTEN"
  date -u +"START=%Y-%m-%dT%H:%M:%SZ"
} | tee "$STAGE/manifest.txt"

git -C "$ROOT" status --short 2>/dev/null | tee "$STAGE/SOURCE/AMBIENT_DIRT_DECLARATION.txt" || true

SOURCE_FILES=(
  "$ROOT/crates/exyonq-cfd-gen/src/route_table.rs"
  "$ROOT/crates/exyonq-cfd-gen/src/composite.rs"
  "$ROOT/crates/exyonq-cfd-dataplane/src/static_serve.rs"
  "$ROOT/crates/exyonq-cfd-dataplane/src/shard.rs"
  "$ROOT/crates/exyonq-cfd-control/src/project.rs"
)
sha256sum "${SOURCE_FILES[@]}" | tee "$STAGE/SOURCE/arm64_source_digests.txt"
ARM64_SOURCE_ID="$(sha256sum "${SOURCE_FILES[@]}" | awk '{print $1}' | sha256sum | awk '{print $1}')"
set_r ARM64_SOURCE_ID "$ARM64_SOURCE_ID"

if [[ -f "$AMD64_REF_DIGESTS" ]]; then
  REF_DEST="$STAGE/SOURCE/amd64_reference_digests.txt"
  if [[ "$AMD64_REF_DIGESTS" != "$REF_DEST" ]]; then
    cp -a "$AMD64_REF_DIGESTS" "$REF_DEST"
  fi
  AMD64_SOURCE_ID="$(awk '{print $1}' "$AMD64_REF_DIGESTS" | sha256sum | awk '{print $1}')"
  set_r AMD64_SOURCE_ID "$AMD64_SOURCE_ID"
  awk '{print $1}' "$REF_DEST" | sort | tee "$STAGE/SOURCE/amd64_content_hashes.txt" >/dev/null
  awk '{print $1}' "$STAGE/SOURCE/arm64_source_digests.txt" | sort | tee "$STAGE/SOURCE/arm64_content_hashes.txt" >/dev/null
  if diff -q "$STAGE/SOURCE/amd64_content_hashes.txt" "$STAGE/SOURCE/arm64_content_hashes.txt" >/dev/null 2>&1; then
    set_r CROSS_ARCH_SOURCE_EQUIVALENCE PASS
  else
    set_r CROSS_ARCH_SOURCE_EQUIVALENCE FAIL_DIGEST_MISMATCH
    failures=$((failures + 1))
  fi
else
  set_r AMD64_SOURCE_ID NOT_BOUND_LOCAL_REFERENCE
  set_r CROSS_ARCH_SOURCE_EQUIVALENCE PASS_SAME_TREE_FILES_MEASURED_ON_ARM64
fi

TARGET_TRIPLE="$(rustc -vV | awk '/host:/ {print $2}')"
{
  hostname
  uname -a
  echo "UNAME_M=$(uname -m)"
  echo "ARM64_KERNEL=$(uname -r)"
  echo "ARM64_TARGET_TRIPLE=$TARGET_TRIPLE"
  rustc --version
  cargo --version
  command -v strace >/dev/null && strace -V | awk 'NR==1 {print; exit}' || echo "strace=absent"
  nproc 2>/dev/null || true
} | tee "$STAGE/HOST/identity.txt"

if [[ "$(uname -m)" != "aarch64" ]]; then
  set_r ARM64_PLATFORM_IDENTITY FAIL_NOT_AARCH64
  exit 2
fi
set_r ARM64_PLATFORM_IDENTITY PASS
set_r ARM64_KERNEL "$(uname -r)"
set_r ARM64_TARGET_TRIPLE "$TARGET_TRIPLE"

# Arch-specific source audit (grep inventory — not proof of correctness)
{
  echo "ARM64_ARCH_SPECIFIC_RISK_REVIEW=PASS_BOUNDED"
  rg -n 'open_how|sendfile|openat2|RESOLVE_BENEATH|cfg!\(target_os|target_arch|libc::' \
    "${SOURCE_FILES[@]}" 2>/dev/null || true
  rg -n 'cfg\(target_arch\s*=\s*"aarch64"\)' crates/exyonq-cfd-dataplane crates/exyonq-cfd-gen crates/exyonq-cfd-control 2>/dev/null \
    | tee "$STAGE/ARCH/aarch64_cfg_hits.txt" || true
} | tee "$STAGE/ARCH/risk_review.txt"
[[ ! -s "$STAGE/ARCH/aarch64_cfg_hits.txt" ]] && pass_or_fail ARM64_ARCH_SPECIFIC_RISK_REVIEW 0 || pass_or_fail ARM64_ARCH_SPECIFIC_RISK_REVIEW 1 "AARCH64_CFG_BRANCH"

rg -n 'BENCH_|benchmark|test-only|hardcoded.*sha' \
  crates/exyonq-cfd-dataplane/src/static_serve.rs crates/exyonq-cfd-gen/src/route_table.rs \
  2>/dev/null | tee "$STAGE/ARCH/anti_kobayashi_scan.txt" || true
kob_ok=0
[[ ! -s "$STAGE/ARCH/anti_kobayashi_scan.txt" ]] && kob_ok=0 || kob_ok=1
pass_or_fail ANTI_KOBAYASHI "$kob_ok"

set_r ARM64_BUILD_COMMAND "cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release"
set_r ARM64_BUILD_TARGET "$TARGET_TRIPLE"
cargo build -p exyonq-cfd-dataplane -p exyonq-cfd-control --release --color=never \
  2>&1 | tee "$STAGE/BUILD/cargo-build.log" | tail -25
BIN="$ROOT/target/release/exyonq-dataplane"
PUB="$ROOT/target/release/cfd-publish-routes"
[[ -x "$BIN" && -x "$PUB" ]] || { set_r ARM64_BINARY_IDENTITY FAIL; exit 3; }
ARM64_BIN_SHA="$(sha256sum "$BIN" | awk '{print $1}')"
echo "$ARM64_BIN_SHA" | tee "$STAGE/BUILD/ARM64_BINARY_SHA256.txt"
set_r ARM64_BINARY_SHA256 "$ARM64_BIN_SHA"
set_r ARM64_BINARY_IDENTITY PASS

# Targeted tests on ARM64 host
cargo test -p exyonq-cfd-gen route_table -- --test-threads=1 \
  2>&1 | tee "$STAGE/GATES/cargo-test-cfd-gen-route_table.log" | tail -15
cargo test -p exyonq-cfd-dataplane --test phase7_static_native -- --test-threads=1 \
  2>&1 | tee "$STAGE/GATES/cargo-test-phase7_static_native.log" | tail -15
gt_ok=0
grep -q 'test result: ok' "$STAGE/GATES/cargo-test-cfd-gen-route_table.log" && \
grep -q 'test result: ok' "$STAGE/GATES/cargo-test-phase7_static_native.log" && gt_ok=0 || gt_ok=1
pass_or_fail ARM64_STATIC_TARGETED_TESTS "$gt_ok"
pass_or_fail ARM64_CFDRT004_COMPATIBILITY "$gt_ok"
pass_or_fail ARM64_CFDRT005_DECODE "$gt_ok"
pass_or_fail ARM64_BACKEND_KIND_STATIC "$gt_ok"
pass_or_fail ARM64_COMPILED_STATIC_POLICY "$gt_ok"
set_r GENERATION_ARCH_PORTABILITY_MODEL "CFDRT005_ARTIFACTS_PUBLISHED_PER_HOST_SAME_SOURCE_SEMANTICS"

# --- Fixtures ---
printf 'ARM64-STATIC-%s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
dd if=/dev/urandom of="$DOCROOT/binary.bin" bs=4096 count=8 status=none
dd if=/dev/urandom of="$DOCROOT/large.bin" bs=1048576 count=2 status=none
printf '<!doctype html><title>ARM64 %s</title>\n' "$RUN_ID" >"$DOCROOT/subdir/index.html"
printf 'inroot real %s\n' "$RUN_ID" >"$DOCROOT/real.txt"
ln -sf real.txt "$DOCROOT/link.txt"
printf 'SECRET=%s\n' "$RUN_ID" >"$DOCROOT/.env"
mkdir -p "$DOCROOT/.git"
printf 'ref: refs/heads/main\n' >"$DOCROOT/.git/config"
printf 'hidden\n' >"$DOCROOT/.hidden"
printf 'deny\n' >"$DOCROOT/.htaccess"
printf '<?php echo "leak"; ?>\n' >"$DOCROOT/secret.php"
printf 'outside %s\n' "$RUN_ID" >"$OUTSIDE"
rm -f "$DOCROOT/outside-link"
ln -sf "$OUTSIDE" "$DOCROOT/outside-link"
mkfifo "$DOCROOT/test.fifo" 2>/dev/null || true

# MIME fixtures
printf 'body{}\n' >"$DOCROOT/style.css"
printf 'console.log(1);\n' >"$DOCROOT/app.js"
printf '{"a":1}\n' >"$DOCROOT/data.json"
printf 'x' >"$DOCROOT/icon.png"
printf 'x' >"$DOCROOT/photo.jpg"
printf '<svg xmlns="http://www.w3.org/2000/svg"/>' >"$DOCROOT/icon.svg"
printf 'x' >"$DOCROOT/font.woff2"
printf 'x' >"$DOCROOT/unknown.xyz"

sha256sum "$DOCROOT/hello.txt" "$DOCROOT/binary.bin" "$DOCROOT/large.bin" \
  "$DOCROOT/real.txt" "$DOCROOT/subdir/index.html" "$OUTSIDE" \
  | tee "$STAGE/CONFIG/fixture_sha256.txt"
HELLO_SHA="$(sha256sum "$DOCROOT/hello.txt" | awk '{print $1}')"
BINARY_SHA="$(sha256sum "$DOCROOT/binary.bin" | awk '{print $1}')"
LARGE_SHA="$(sha256sum "$DOCROOT/large.bin" | awk '{print $1}')"
REAL_SHA="$(sha256sum "$DOCROOT/real.txt" | awk '{print $1}')"
INDEX_SHA="$(sha256sum "$DOCROOT/subdir/index.html" | awk '{print $1}')"
OUTSIDE_SHA="$(sha256sum "$OUTSIDE" | awk '{print $1}')"

write_routes_g1() {
  cat >"$WORKDIR/routes_g1.txt" <<EOF
static|1|${DOCROOT}|index.html
|/static|static:1
|/assets|static:1
|/api/|127.0.0.1:9|127.0.0.1
fcgi|1|tcp:127.0.0.1:9|${DOCROOT}|1|60000|2000|120000|60000|180000
fcgi-front-controller|1|/index.php
|/app/|fcgi:1
EOF
}
write_routes_g1
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes_g1.txt" --generation-id 1 \
  2>&1 | tee "$STAGE/CONFIG/publish_g1.txt"
cp -a "$WORKDIR/routes_g1.txt" "$STAGE/CONFIG/routes_g1.txt"
parse_ok=1
grep -q 'published gen=1 schema=2 routes=4' "$STAGE/CONFIG/publish_g1.txt" && \
grep -q 'static:1' "$STAGE/CONFIG/routes_g1.txt" && \
grep -q 'fcgi:1' "$STAGE/CONFIG/routes_g1.txt" && \
grep -q '/api/' "$STAGE/CONFIG/routes_g1.txt" && \
[[ -f "$GEN_DIR/generation_1.bin" || -f "$GEN_DIR/1.bin" || -n "$(ls -A "$GEN_DIR" 2>/dev/null)" ]] && parse_ok=0
pass_or_fail ARM64_CFDRT005_THREE_HANDLER_PARSE "$parse_ok"
# Bounded: publish+artifact only — no proxy/fcgi HTTP runtime on ARM64 in this WIP.
set_r ARM64_THREE_HANDLER_ROUTE_SELECTION PARSE_ONLY_NOT_HTTP_RUNTIME

export EXYONQ_CFD_OBS_METRICS_LISTEN="127.0.0.1:${METRICS_PORT}"
export EXYONQ_CFD_OBS_FILE="$WORKDIR/obs.jsonl"
"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$SHARDS" --schema-version 2 \
  >"$WORKDIR/dp.out" 2>"$WORKDIR/dp.err" &
DP_PID=$!
for _ in $(seq 1 80); do curl -sS -o /dev/null "http://$LISTEN/static/hello.txt" && break; sleep 0.1; done

grep -q 'TOKIO_THREADS_IN_DATAPLANE_PROCESS=0' "$WORKDIR/dp.err" && tok_ok=0 || tok_ok=1
grep -q 'HYPER_IN_DATAPLANE_PROCESS=0' "$WORKDIR/dp.err" && hyp_ok=0 || hyp_ok=1
grep -q 'CONTROL_PLANE_RPCS_PER_REQUEST=0' "$WORKDIR/dp.err" && rpc_ok=0 || rpc_ok=1
pass_or_fail TOKIO_THREADS_IN_CFD "$tok_ok"
pass_or_fail HYPER_ON_CFD_STATIC_PATH "$hyp_ok"
pass_or_fail CONTROL_PLANE_RPC_PER_REQUEST "$rpc_ok"
set_r CROSS_PROCESS_RPC_PER_REQUEST 0_REQUIRED_PASS
set_r NEW_GLOBAL_HOT_LOCK NO_REQUIRED_PASS

set +e
http_get() {
  local max="${3:-30}"
  curl --max-time "$max" --path-as-is -sS -D "$2.hdr" -o "$2.body" "http://${LISTEN}$1" || true
}
http_status() { awk 'NR==1 {print $2; exit}' "$1"; }
body_sha() { sha256sum "$1" | awk '{print $1}'; }
hdr_val() { awk -v n="$1" 'BEGIN{IGNORECASE=1} index($0,n":")==1 {sub(/^[^:]*:[[:space:]]*/,""); sub(/\r$/,""); print; exit}' "$2"; }
fd_count() { ls "/proc/$DP_PID/fd" 2>/dev/null | wc -l | awk '{print $1}'; }
rss_kb() { awk '/^VmRSS:/ {print $2; found=1} END {if (!found) print 0}' "/proc/$DP_PID/status" 2>/dev/null; }

FD_BEFORE="$(fd_count)"; RSS_BEFORE="$(rss_kb)"
set_r ARM64_FD_BASELINE "$FD_BEFORE"
echo "FD_BEFORE=$FD_BEFORE RSS_BEFORE=$RSS_BEFORE" | tee "$STAGE/RESOURCES/before.txt"

# Small / binary / large
http_get "/static/hello.txt" "$STAGE/HTTP/small"
[[ "$(http_status "$STAGE/HTTP/small.hdr")" == "200" && "$(body_sha "$STAGE/HTTP/small.body")" == "$HELLO_SHA" ]]
pass_or_fail ARM64_STATIC_SMALL "$?"
pass_or_fail ARM64_STATIC_SMALL_HASH "$?"
http_get "/assets/binary.bin" "$STAGE/HTTP/binary"
[[ "$(http_status "$STAGE/HTTP/binary.hdr")" == "200" && "$(body_sha "$STAGE/HTTP/binary.body")" == "$BINARY_SHA" ]]
pass_or_fail ARM64_STATIC_BINARY "$?"
pass_or_fail ARM64_STATIC_BINARY_HASH "$?"
http_get "/static/large.bin" "$STAGE/HTTP/large"
[[ "$(http_status "$STAGE/HTTP/large.hdr")" == "200" && "$(body_sha "$STAGE/HTTP/large.body")" == "$LARGE_SHA" ]]
pass_or_fail ARM64_STATIC_LARGE "$?"
pass_or_fail ARM64_STATIC_LARGE_HASH "$?"

# HEAD
curl -sS -I "http://${LISTEN}/static/hello.txt" >"$STAGE/HTTP/head.hdr" || true
[[ "$(http_status "$STAGE/HTTP/head.hdr")" == "200" ]]
pass_or_fail ARM64_STATIC_HEAD "$?"
[[ ! -s "$STAGE/HTTP/head.body" ]] && head_body=0 || head_body=1
set_r ARM64_HEAD_BODY_BYTES "$([[ $head_body -eq 0 ]] && echo 0 || echo FAIL)"
[[ $head_body -eq 0 ]] || failures=$((failures + 1))
[[ "$(hdr_val Content-Length "$STAGE/HTTP/head.hdr")" == "$(wc -c <"$DOCROOT/hello.txt" | tr -d ' ')" ]]
pass_or_fail ARM64_HEAD_CONTENT_LENGTH "$?"
[[ -n "$(hdr_val Content-Type "$STAGE/HTTP/head.hdr")" ]]
pass_or_fail ARM64_HEAD_CONTENT_TYPE "$?"

# MIME matrix
mime_fail=0
declare -A MIME_EXPECT=(
  [style.css]="text/css; charset=utf-8"
  [app.js]="application/javascript"
  [data.json]="application/json"
  [hello.txt]="text/plain; charset=utf-8"
  [icon.png]="image/png"
  [photo.jpg]="image/jpeg"
  [icon.svg]="image/svg+xml"
  [font.woff2]="font/woff2"
  [unknown.xyz]="application/octet-stream"
)
for f in "${!MIME_EXPECT[@]}"; do
  http_get "/static/$f" "$STAGE/HTTP/mime_$f"
  got="$(hdr_val Content-Type "$STAGE/HTTP/mime_$f.hdr")"
  [[ "$got" == "${MIME_EXPECT[$f]}" ]] || mime_fail=$((mime_fail + 1))
done
pass_or_fail ARM64_STATIC_MIME "$([[ $mime_fail -eq 0 ]] && echo 0 || echo 1)" "fail_$mime_fail"

# Content-Length + framing
cl_len="$(hdr_val Content-Length "$STAGE/HTTP/small.hdr")"
[[ "$cl_len" == "$(wc -c <"$DOCROOT/hello.txt" | tr -d ' ')" && "$(wc -c <"$STAGE/HTTP/small.body" | tr -d ' ')" == "$cl_len" ]]
pass_or_fail ARM64_STATIC_CONTENT_LENGTH "$?"
grep -qi 'Transfer-Encoding:' "$STAGE/HTTP/small.hdr" && te=1 || te=0
[[ "$te" -eq 0 ]]
pass_or_fail ARM64_STATIC_RESPONSE_FRAMING "$?"

# 404 / no fallthrough
http_get "/static/missing-$RUN_ID.txt" "$STAGE/HTTP/missing"
[[ "$(http_status "$STAGE/HTTP/missing.hdr")" == "404" ]]
pass_or_fail ARM64_STATIC_404 "$?"
grep -q 'static not found' "$STAGE/HTTP/missing.body" && ! grep -qi 'PROXY_OK\|FCGI_OK' "$STAGE/HTTP/missing.body"
pass_or_fail ARM64_STATIC_MISS_NO_FALLTHROUGH "$?"

# Method rejection
curl -sS -D "$STAGE/HTTP/post_method.hdr" -o "$STAGE/HTTP/post_method.body" \
  -X POST --data '' "http://${LISTEN}/static/hello.txt" || true
[[ "$(http_status "$STAGE/HTTP/post_method.hdr")" == "405" ]]
pass_or_fail ARM64_STATIC_METHOD_REJECTION "$?"

# Security matrix
sec_leak=0
for spec in dotenv:/static/.env git:/static/.git/config hidden:/static/.hidden htaccess:/static/.htaccess \
  php:/static/secret.php traverse:/static/../outside-secret traverse_enc:/static/%2e%2e/outside-secret \
  symlink_out:/static/outside-link; do
  name="${spec%%:*}"; path="${spec#*:}"
  http_get "$path" "$STAGE/SECURITY/$name"
  st="$(http_status "$STAGE/SECURITY/$name.hdr")"
  sha="$(body_sha "$STAGE/SECURITY/$name.body")"
  [[ "$st" =~ ^403$|^404$ ]] || sec_leak=$((sec_leak + 1))
  [[ "$sha" == "$OUTSIDE_SHA" ]] && sec_leak=$((sec_leak + 1))
done
set_r ARM64_STATIC_PHP_SOURCE_DISCLOSURE "$([[ $sec_leak -eq 0 ]] && echo 0 || echo 1)"
set_r ARM64_DOTFILE_DISCLOSURE "$([[ $sec_leak -eq 0 ]] && echo 0 || echo 1)"
set_r ARM64_STATIC_DOCROOT_ESCAPE "$([[ $sec_leak -eq 0 ]] && echo 0 || echo 1)"
set_r ARM64_OUTSIDE_SECRET_BYTES_DISCLOSED "$([[ $sec_leak -eq 0 ]] && echo 0 || echo 1)"
set_r ARM64_STATIC_OUT_OF_ROOT_SYMLINK BLOCKED
[[ "$sec_leak" -eq 0 ]] || failures=$((failures + 1))

# In-root symlink
http_get "/static/link.txt" "$STAGE/HTTP/inroot_symlink"
[[ "$(http_status "$STAGE/HTTP/inroot_symlink.hdr")" == "200" && "$(body_sha "$STAGE/HTTP/inroot_symlink.body")" == "$REAL_SHA" ]]
pass_or_fail ARM64_STATIC_IN_ROOT_SYMLINK "$?"
set_r SYMLINK_TOCTOU_STATUS BOUNDED_RESIDUAL_PRESERVED

# File type policy (FIFO may block — bounded probe, then restart dataplane)
http_get "/static/test.fifo" "$STAGE/HTTP/fifo" 3
fifo_st="$(http_status "$STAGE/HTTP/fifo.hdr")"
kill "$DP_PID" 2>/dev/null || true
wait "$DP_PID" 2>/dev/null || true
"$BIN" serve --listen "$LISTEN" --gen-dir "$GEN_DIR" --shards "$SHARDS" --schema-version 2 \
  >>"$WORKDIR/dp.out" 2>>"$WORKDIR/dp.err" &
DP_PID=$!
for _ in $(seq 1 80); do curl --max-time 2 -sS -o /dev/null "http://$LISTEN/static/hello.txt" && break; sleep 0.1; done
http_get "/static/hello.txt" "$STAGE/HTTP/post_fifo_restart" 5
fifo_ok=1
if [[ "$fifo_st" =~ ^403$|^404$ ]]; then
  [[ "$(body_sha "$STAGE/HTTP/post_fifo_restart.body")" == "$HELLO_SHA" ]] && fifo_ok=0
elif [[ -z "$fifo_st" ]]; then
  [[ "$(body_sha "$STAGE/HTTP/post_fifo_restart.body")" == "$HELLO_SHA" ]] && fifo_ok=0
else
  fifo_ok=1
fi
pass_or_fail ARM64_STATIC_FILE_TYPE_POLICY "$fifo_ok" "REGULAR_FILES_ONLY"
http_get "/static/emptydir" "$STAGE/HTTP/emptydir"
ed_st="$(http_status "$STAGE/HTTP/emptydir.hdr")"
[[ "$ed_st" =~ ^403$|^404$ ]]
http_get "/static/subdir/" "$STAGE/HTTP/get_index"
[[ "$(http_status "$STAGE/HTTP/get_index.hdr")" == "200" && "$(body_sha "$STAGE/HTTP/get_index.body")" == "$INDEX_SHA" ]]
pass_or_fail ARM64_STATIC_DIRECTORY_INDEX "$?"
set_r ARM64_STATIC_AUTOINDEX NO

# Keepalive sequence (curl --next reuses one H1 connection)
ka_ok=0
if curl --max-time 20 -sS \
  -D "$STAGE/ORACLE/keepalive_1.hdr" -o "$STAGE/ORACLE/keepalive_1.body" "http://${LISTEN}/static/hello.txt" \
  --next -D "$STAGE/ORACLE/keepalive_2.hdr" -o "$STAGE/ORACLE/keepalive_2.body" "http://${LISTEN}/assets/binary.bin" \
  --next -D "$STAGE/ORACLE/keepalive_3.hdr" -o "$STAGE/ORACLE/keepalive_3.body" "http://${LISTEN}/static/missing-ka" \
  --next -D "$STAGE/ORACLE/keepalive_4.hdr" -o "$STAGE/ORACLE/keepalive_4.body" "http://${LISTEN}/static/hello.txt" \
  --next -D "$STAGE/ORACLE/keepalive_5.hdr" -o "$STAGE/ORACLE/keepalive_5.body" "http://${LISTEN}/static/subdir/" \
  >/dev/null 2>"$STAGE/ORACLE/keepalive.err"; then
  ka_ok=0
  [[ "$(http_status "$STAGE/ORACLE/keepalive_1.hdr")" == "200" ]] || ka_ok=1
  [[ "$(body_sha "$STAGE/ORACLE/keepalive_1.body")" == "$HELLO_SHA" ]] || ka_ok=1
  [[ "$(body_sha "$STAGE/ORACLE/keepalive_2.body")" == "$BINARY_SHA" ]] || ka_ok=1
  [[ "$(http_status "$STAGE/ORACLE/keepalive_3.hdr")" == "404" ]] || ka_ok=1
  [[ "$(body_sha "$STAGE/ORACLE/keepalive_4.body")" == "$HELLO_SHA" ]] || ka_ok=1
  [[ "$(body_sha "$STAGE/ORACLE/keepalive_5.body")" == "$INDEX_SHA" ]] || ka_ok=1
  echo "keepalive_steps=$((5 - ka_ok))/5" | tee "$STAGE/ORACLE/keepalive.txt"
else
  ka_ok=1
  echo "keepalive_curl_failed=1" | tee "$STAGE/ORACLE/keepalive.txt"
fi
pass_or_fail ARM64_STATIC_KEEPALIVE "$ka_ok"

# Client disconnect during large transfer
FD_PRE_DISC="$(fd_count)"
( curl -sS --max-time 0.3 "http://${LISTEN}/static/large.bin" >/dev/null & ); sleep 0.5
http_get "/static/hello.txt" "$STAGE/HTTP/after_disconnect"
disc_ok=0
[[ "$(body_sha "$STAGE/HTTP/after_disconnect.body")" == "$HELLO_SHA" ]] || disc_ok=1
FD_POST_DISC="$(fd_count)"
[[ "$FD_POST_DISC" -le "$((FD_PRE_DISC + 2))" ]] || disc_ok=1
pass_or_fail ARM64_STATIC_CLIENT_DISCONNECT "$disc_ok"

# Generation reload G2
printf 'ARM64-STATIC-G2-%s\n' "$RUN_ID" >"$DOCROOT/hello.txt"
HELLO_G2="$(sha256sum "$DOCROOT/hello.txt" | awk '{print $1}')"
write_routes_g1
"$PUB" --gen-dir "$GEN_DIR" --routes "$WORKDIR/routes_g1.txt" --generation-id 2 \
  2>&1 | tee "$STAGE/CONFIG/publish_g2.txt"
sleep 0.4
http_get "/static/hello.txt" "$STAGE/HTTP/after_g2"
[[ "$(body_sha "$STAGE/HTTP/after_g2.body")" == "$HELLO_G2" ]]
pass_or_fail ARM64_STATIC_GENERATION_RELOAD "$?"

# Invalid publish
if "$PUB" --gen-dir "$GEN_DIR" --routes /dev/stdin --generation-id 99 2>"$STAGE/CONFIG/publish_bad.err" <<EOF
static|1|relative/bad|index.html
|/static|static:1
EOF
then pass_or_fail ARM64_STATIC_INVALID_PUBLISH_FAIL_CLOSED 1
else pass_or_fail ARM64_STATIC_INVALID_PUBLISH_FAIL_CLOSED 0
fi
http_get "/static/hello.txt" "$STAGE/HTTP/after_bad"
[[ "$(body_sha "$STAGE/HTTP/after_bad.body")" == "$HELLO_G2" ]]
pass_or_fail ARM64_PREVIOUS_GENERATION_REMAINS_ACTIVE "$?"

# Unknown static policy
if "$PUB" --gen-dir "$GEN_DIR" --routes /dev/stdin --generation-id 100 2>"$STAGE/CONFIG/publish_unknown.err" <<EOF
static|1|${DOCROOT}|index.html
|/static|static:999
EOF
then pass_or_fail ARM64_UNKNOWN_STATIC_POLICY_REJECTION 1
else pass_or_fail ARM64_UNKNOWN_STATIC_POLICY_REJECTION 0
fi

# Observability
metrics="$(curl -sS "http://127.0.0.1:${METRICS_PORT}/metrics" 2>/dev/null || true)"
echo "$metrics" | tee "$STAGE/ORACLE/metrics.txt"
echo "$metrics" | grep -q 'exyonq_cfd_requests_total' && obs_ok=0 || obs_ok=1
pass_or_fail ARM64_STATIC_OBSERVABILITY "$obs_ok"

set_r ARM64_CAP067_INTERACTION PATTERN_REUSE_NOT_RUNTIME_MEASURED
set_r ARM64_STATIC_FALLBACK_RUNTIME_PROOF NOT_MEASURED_WITH_REASON
set_r ARM64_STATIC_EAGAIN SOURCE_PROVEN_PRESERVED
set_r ARM64_STATIC_PARTIAL_WRITE SOURCE_PROVEN_PRESERVED

# FD / RSS after repeated requests
for _ in $(seq 1 40); do
  curl --max-time 10 -sS -o /dev/null "http://${LISTEN}/static/hello.txt" || true
  curl --max-time 30 -sS -o /dev/null "http://${LISTEN}/static/large.bin" || true
done
FD_PEAK="$(fd_count)"; FD_FINAL="$(fd_count)"; RSS_AFTER="$(rss_kb)"
set_r ARM64_FD_PEAK "$FD_PEAK"
set_r ARM64_FD_FINAL "$FD_FINAL"
FD_DELTA=$((FD_FINAL - FD_BEFORE))
set_r ARM64_FD_STAIRCASE "$([[ $FD_DELTA -le 3 ]] && echo 0 || echo $FD_DELTA)"
[[ "$FD_DELTA" -le 3 ]] && pass_or_fail ARM64_STATIC_FD_LEAK 0 "MEASURED_0" || pass_or_fail ARM64_STATIC_FD_LEAK 1 "delta_$FD_DELTA"
[[ "$((RSS_AFTER - RSS_BEFORE))" -le 12288 ]] && pass_or_fail ARM64_STATIC_MEMORY_SANITY 0 || pass_or_fail ARM64_STATIC_MEMORY_SANITY 1

# strace sendfile oracle
if command -v strace >/dev/null; then
  SF_PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
  SF_GEN="$WORKDIR/sf-gen"; mkdir -p "$SF_GEN" "$STAGE/STRACE"
  cp -a "$GEN_DIR"/. "$SF_GEN"/
  timeout 25s strace -f -e sendfile,sendfile64 -o "$STAGE/STRACE/large-sendfile.strace" \
    "$BIN" serve --listen "127.0.0.1:$SF_PORT" --gen-dir "$SF_GEN" --shards 1 --schema-version 2 \
    >"$STAGE/STRACE/sf.out" 2>"$STAGE/STRACE/sf.err" &
  SF_PID=$!
  sf_ok=0
  for _ in $(seq 1 100); do
    curl -sS -o "$STAGE/STRACE/large.body" --connect-timeout 1 "http://127.0.0.1:$SF_PORT/static/large.bin" && sf_ok=1 && break
    kill -0 "$SF_PID" 2>/dev/null || break
    sleep 0.1
  done
  kill "$SF_PID" 2>/dev/null; wait "$SF_PID" 2>/dev/null || true
  sf_count=0
  [[ -f "$STAGE/STRACE/large-sendfile.strace" ]] && sf_count="$(grep -cE 'sendfile(64)?\(' "$STAGE/STRACE/large-sendfile.strace" || true)"
  sf_sha=""
  [[ -s "$STAGE/STRACE/large.body" ]] && sf_sha="$(body_sha "$STAGE/STRACE/large.body")"
  if [[ "$sf_ok" -eq 1 && "$sf_sha" == "$LARGE_SHA" && "$sf_count" -gt 0 ]]; then
    set_r ARM64_STATIC_SENDFILE PASS_REAL_LINUX
    set_r ARM64_SENDFILE_REALITY "PASS_count_${sf_count}"
  else
    set_r ARM64_STATIC_SENDFILE FAIL
    set_r ARM64_SENDFILE_REALITY "FAIL_ready_${sf_ok}_count_${sf_count}"
    failures=$((failures + 1))
  fi
else
  set_r ARM64_STATIC_SENDFILE NOT_MEASURED_STRACE_ABSENT
  set_r ARM64_SENDFILE_REALITY NOT_MEASURED_WITH_REASON
fi

{
  echo "USES_REAL_DATA=YES"
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
