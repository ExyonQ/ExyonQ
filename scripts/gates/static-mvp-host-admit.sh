#!/usr/bin/env bash
# Static-mvp dual-Linux host admission runner (tracked source).
# Consumed remotely after packaging copies this file into a WIP temp transfer.
#
# Args: HOST_LABEL EXPECTED_ARCH WORK_ROOT
#
# Invariants:
# - Unique WIP EXYONQ_CONTROL_SOCKET (never host-shared /tmp/exyonq.sock).
# - Never delete or mutate foreign sockets under /tmp.
# - Product paths verified via candidate.manifest before build.
set -euo pipefail
HOST_LABEL="${1:?}"
EXPECTED_ARCH="${2:?}"
WORK_ROOT="${3:?}"

source ~/.cargo/env 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"

EV="$WORK_ROOT/evidence"
SRC="$WORK_ROOT/src"
mkdir -p "$EV"/{meta,static-mvp,reject,core-gate,logs}
RESULT="$EV/result.env"
: >"$RESULT"

log() { printf '%s\n' "$*" | tee -a "$EV/logs/console.log"; }

fail_stop() {
  local attr="$1" msg="$2"
  {
    echo "HOST_RESULT=FAIL"
    echo "FAILURE_ATTRIBUTION=$attr"
    echo "FAILURE_MESSAGE=$msg"
    echo "ENDED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } | tee -a "$RESULT"
  log "FAIL_STOP attr=$attr msg=$msg"
  exit 1
}

block_stop() {
  local attr="$1" msg="$2"
  {
    echo "HOST_RESULT=BLOCKED"
    echo "FAILURE_ATTRIBUTION=$attr"
    echo "FAILURE_MESSAGE=$msg"
    echo "ENDED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } | tee -a "$RESULT"
  log "BLOCK_STOP attr=$attr msg=$msg"
  exit 3
}

# --- preflight ---
ARCH="$(uname -m)"
OS="$(uname -s)"
[[ "$OS" == "Linux" ]] || block_stop ENVIRONMENT "not Linux: $OS"
case "$EXPECTED_ARCH" in
  amd64|x86_64) [[ "$ARCH" == "x86_64" ]] || block_stop ENVIRONMENT "arch want x86_64 got $ARCH" ;;
  arm64|aarch64) [[ "$ARCH" == "aarch64" ]] || block_stop ENVIRONMENT "arch want aarch64 got $ARCH" ;;
  *) block_stop HARNESS "unknown expected arch $EXPECTED_ARCH" ;;
esac

command -v rustc >/dev/null || block_stop ENVIRONMENT "missing rustc"
command -v cargo >/dev/null || block_stop ENVIRONMENT "missing cargo"
PHP_FPM=""
for c in php-fpm8.3 php-fpm8.2 php-fpm php-fpm8.1; do
  if command -v "$c" >/dev/null 2>&1; then PHP_FPM="$(command -v "$c")"; break; fi
done
[[ -n "$PHP_FPM" ]] || block_stop ENVIRONMENT "missing php-fpm"

{
  echo "HOST_LABEL=$HOST_LABEL"
  echo "ARCH=$ARCH"
  echo "OS=$OS"
  echo "RUSTC=$(rustc -V)"
  echo "CARGO=$(cargo -V)"
  echo "PHP_FPM=$PHP_FPM"
  echo "WORK_ROOT=$WORK_ROOT"
  echo "STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee "$EV/meta/host.env"

# --- verify candidate manifest ---
MANIFEST="$WORK_ROOT/candidate.manifest"
[[ -f "$MANIFEST" ]] || fail_stop HARNESS "missing candidate.manifest"
while read -r kind hash path; do
  [[ "$kind" == "CANDIDATE" ]] || continue
  f="$SRC/$path"
  [[ -f "$f" ]] || fail_stop HARNESS "missing candidate file $path"
  got="$(sha256sum "$f" | awk '{print $1}')"
  if [[ "$got" != "$hash" ]]; then
    fail_stop HARNESS "hash mismatch $path want=$hash got=$got"
  fi
done < <(grep '^CANDIDATE ' "$MANIFEST")
log "MANIFEST_VERIFY=PASS"

# --- build ---
cd "$SRC"
export CARGO_TARGET_DIR="$WORK_ROOT/target"
log "BUILD release locked start"
if ! cargo build --release --locked -p exyonq -p exyonqctl >"$EV/logs/build.stdout" 2>"$EV/logs/build.stderr"; then
  fail_stop PRODUCT "cargo build --release --locked failed"
fi
EXY="$CARGO_TARGET_DIR/release/exyonq"
CTL="$CARGO_TARGET_DIR/release/exyonqctl"
[[ -x "$EXY" && -x "$CTL" ]] || fail_stop PRODUCT "missing release binaries"
BIN_SHA="$(sha256sum "$EXY" | awk '{print $1}')"
echo "BINARY_SHA256=$BIN_SHA" | tee -a "$EV/meta/host.env"
log "BUILD=PASS binary_sha=$BIN_SHA"

# Isolated control socket for this WIP only (never /tmp/exyonq.sock host-shared default).
# Short path under /tmp for AF_UNIX sockaddr limits; name embeds WIP id.
# Drop any inherited host/default socket path before binding ours.
unset EXYONQ_CONTROL_SOCKET || true
CTRL_SOCK="/tmp/exyonq-smvp-ctrl-${HOST_LABEL}-$$.sock"
if [[ -e "$CTRL_SOCK" ]]; then
  fail_stop HARNESS "WIP control socket path already exists: $CTRL_SOCK"
fi
export EXYONQ_CONTROL_SOCKET="$CTRL_SOCK"
echo "EXYONQ_CONTROL_SOCKET=$CTRL_SOCK" | tee -a "$EV/meta/host.env"
log "CONTROL_SOCKET_ISOLATED=$CTRL_SOCK"

# --- static-mvp import + serve ---
SM="$EV/static-mvp"
mkdir -p "$SM/www" "$SM/reject"
printf 'hello-static-mvp\n' >"$SM/www/index.html"
printf 'BINDATA' >"$SM/www/blob.bin"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
cat >"$SM/nginx-accept.conf" <<EOF
server {
    listen 127.0.0.1:${PORT};
    server_name static-mvp.local;
    root $SM/www;
    index index.html;
}
EOF

set +e
"$CTL" config migrate-nginx --input "$SM/nginx-accept.conf" --profile static-mvp \
  --write --output "$SM/exyonq.toml" --report "$SM/accept-report.txt" \
  >"$SM/accept.stdout" 2>"$SM/accept.stderr"
ACC_EC=$?
set -e
echo "ACCEPT_EXIT=$ACC_EC" | tee -a "$SM/accept.env"
if [[ "$ACC_EC" -ne 0 || ! -s "$SM/exyonq.toml" ]]; then
  fail_stop PRODUCT "static-mvp accept migrate failed ec=$ACC_EC"
fi
grep -q 'root =' "$SM/exyonq.toml" || fail_stop PRODUCT "IR missing root"
log "STATIC_MVP_IMPORT_MIGRATE=PASS"

# serve with isolated control socket + config binding
export EXYONQ_CONFIG="$SM/exyonq.toml"
EXYONQ_CONFIG="$SM/exyonq.toml" EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" \
  "$EXY" serve --config "$SM/exyonq.toml" >"$SM/serve.stdout" 2>"$SM/serve.stderr" &
SPID=$!
echo "SERVE_PID=$SPID" >>"$SM/accept.env"
echo "CONTROL_SOCKET=$CTRL_SOCK" >>"$SM/accept.env"
READY=0
for _ in $(seq 1 80); do
  if curl -sf -o /dev/null "http://127.0.0.1:${PORT}/"; then READY=1; break; fi
  if ! kill -0 "$SPID" 2>/dev/null; then
    fail_stop PRODUCT "serve exited before ready"
  fi
  sleep 0.25
done
[[ "$READY" -eq 1 ]] || fail_stop PRODUCT "serve readiness timeout"
# Foreign default socket may exist; our process must use CTRL_SOCK only.
[[ -S "$CTRL_SOCK" ]] || fail_stop HARNESS "isolated control socket not created: $CTRL_SOCK"
log "CONTROL_SOCKET_BOUND=$CTRL_SOCK"

GET_BODY="$(curl -sS "http://127.0.0.1:${PORT}/")"
GET_CODE="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/")"
HEAD_CODE="$(curl -sS -I -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/")"
BIN_BODY="$(curl -sS "http://127.0.0.1:${PORT}/blob.bin")"
BIN_CODE="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/blob.bin")"
MISS_CODE="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/missing-nope.html")"
{
  echo "GET_CODE=$GET_CODE"
  echo "HEAD_CODE=$HEAD_CODE"
  echo "BIN_CODE=$BIN_CODE"
  echo "MISS_CODE=$MISS_CODE"
  echo "GET_BODY=$GET_BODY"
  echo "BIN_BODY=$BIN_BODY"
} | tee "$SM/http.env"

HTTP_OK=1
[[ "$GET_CODE" == "200" && "$GET_BODY" == "hello-static-mvp" ]] || HTTP_OK=0
[[ "$HEAD_CODE" == "200" ]] || HTTP_OK=0
[[ "$BIN_CODE" == "200" && "$BIN_BODY" == "BINDATA" ]] || HTTP_OK=0
[[ "$MISS_CODE" == "404" ]] || HTTP_OK=0

kill -TERM "$SPID" 2>/dev/null || true
wait "$SPID" 2>/dev/null || true
rm -f "$CTRL_SOCK"
if grep -Eiq "thread '.*' panicked|panicked at|fatal runtime error|SIGSEGV|Aborted \\(core dumped\\)" \
  "$SM/serve.stderr" "$SM/serve.stdout" 2>/dev/null; then
  fail_stop PRODUCT "process crash during static-mvp serve"
fi
[[ "$HTTP_OK" -eq 1 ]] || fail_stop PRODUCT "HTTP oracle failed"
echo "STATIC_MVP_IMPORT=PASS" | tee -a "$RESULT"
echo "CONTROL_SOCKET_ISOLATED=PASS path=$CTRL_SOCK" | tee -a "$EV/meta/host.env"
if [[ "$HOST_LABEL" == "oracle" ]]; then
  echo "ORACLE_CONTROL_SOCKET_ISOLATED=PASS" | tee -a "$EV/meta/host.env"
  echo "ORACLE_CONTROL_SOCKET_PATH=$CTRL_SOCK" | tee -a "$EV/meta/host.env"
fi
log "STATIC_MVP_IMPORT=PASS"

# --- rejection matrix ---
cat >"$SM/reject/proxy.conf" <<'EOF'
server {
    listen 80;
    root /srv/www;
    location / { proxy_pass http://127.0.0.1:3000; }
}
EOF
cat >"$SM/reject/fcgi.conf" <<'EOF'
server {
    listen 80;
    root /srv/php;
    location / { fastcgi_pass unix:/run/php/php-fpm.sock; }
}
EOF
cat >"$SM/reject/include.conf" <<'EOF'
server {
    listen 80;
    root /srv/www;
    include mime.types;
}
EOF
cat >"$SM/reject/unknown.conf" <<'EOF'
server {
    listen 80;
    root /srv/www;
    gzip on;
}
EOF

REJ_OK=1
for name in proxy fcgi include unknown; do
  rm -f "$SM/reject/${name}.toml"
  set +e
  "$CTL" config migrate-nginx --input "$SM/reject/${name}.conf" --profile static-mvp \
    --write --output "$SM/reject/${name}.toml" \
    >"$SM/reject/${name}.stdout" 2>"$SM/reject/${name}.stderr"
  EC=$?
  set -e
  SZ=0
  [[ -f "$SM/reject/${name}.toml" ]] && SZ=$(wc -c <"$SM/reject/${name}.toml" | tr -d ' ')
  echo "REJECT_${name}_EXIT=$EC OUT_BYTES=$SZ" | tee -a "$SM/reject/matrix.env"
  if [[ "$EC" -eq 0 || "$SZ" -ne 0 ]]; then
    REJ_OK=0
  fi
done
[[ "$REJ_OK" -eq 1 ]] || fail_stop PRODUCT "rejection matrix failed"
echo "REJECTION_MATRIX=PASS" | tee -a "$RESULT"
log "REJECTION_MATRIX=PASS"

# --- core regression gate ---
export EXYONQ_BIN="$EXY"
export EXYONQCTL_BIN="$CTL"
export CARGO_TARGET_DIR
export PHP_FPM_BIN="$PHP_FPM"
# Fresh isolated control socket for gate children (do not inherit host /tmp/exyonq.sock).
GATE_CTRL="/tmp/exyonq-smvp-gate-${HOST_LABEL}-$$.sock"
rm -f "$GATE_CTRL"
export EXYONQ_CONTROL_SOCKET="$GATE_CTRL"
echo "GATE_EXYONQ_CONTROL_SOCKET=$GATE_CTRL" | tee -a "$EV/meta/host.env"
chmod +x "$SRC/scripts/gates/core-regression-gate.sh" \
  "$SRC/scripts/gates/static-mvp-host-admit.sh" \
  "$SRC/scripts/e2e/"*.sh "$SRC/scripts/e2e/suites/"*.sh 2>/dev/null || true

CG_EV="$EV/core-gate/run"
set +e
bash "$SRC/scripts/gates/core-regression-gate.sh" \
  --evidence-root "$CG_EV" \
  --run-id "${HOST_LABEL}-coregate" \
  >"$EV/core-gate/console.stdout" 2>"$EV/core-gate/console.stderr"
CG_EC=$?
set -e
rm -f "$GATE_CTRL"
echo "CORE_GATE_EXIT=$CG_EC" | tee -a "$EV/core-gate/exit.env"
if [[ -f "$CG_EV/summary/axes.env" ]]; then
  cp "$CG_EV/summary/axes.env" "$EV/core-gate/axes.env"
  cat "$CG_EV/summary/axes.env" | tee -a "$RESULT"
else
  fail_stop HARNESS "core gate missing axes.env"
fi
if [[ -f "$CG_EV/summary/result.env" ]]; then
  cp "$CG_EV/summary/result.env" "$EV/core-gate/result.env"
fi
if [[ "$CG_EC" -ne 0 ]]; then
  if [[ "$CG_EC" -eq 3 ]]; then
    block_stop ENVIRONMENT "core gate exit=$CG_EC"
  fi
  fail_stop PRODUCT "core gate exit=$CG_EC"
fi
if [[ -f "$CG_EV/summary/result.env" ]]; then
  # shellcheck disable=SC1091
  source "$CG_EV/summary/result.env"
  if [[ "${GATE_VERDICT:-}" != "PASS" ]]; then
    fail_stop PRODUCT "core gate GATE_VERDICT=${GATE_VERDICT:-MISSING}"
  fi
fi

# Require all axes PASS
# shellcheck disable=SC1091
source "$CG_EV/summary/axes.env"
for ax in CORE_STATIC CORE_PROXY CORE_FASTCGI CORE_OPERATIONS; do
  val="${!ax}"
  echo "${ax}=$val" | tee -a "$RESULT"
  if [[ "$val" != "PASS" ]]; then
    if [[ "$val" == "BLOCKED" || "$val" == "NOT_EXECUTED" ]]; then
      block_stop ENVIRONMENT "core gate axis $ax=$val"
    fi
    fail_stop PRODUCT "core gate axis $ax=$val"
  fi
done

# Real process crash signatures only (not PASS prose containing the word panic).
set +e
grep -REiq "thread '.*' panicked|panicked at|fatal runtime error|SIGSEGV|Aborted \\(core dumped\\)" \
  "$EV/static-mvp" "$EV/core-gate/run/cases" 2>/dev/null
PANIC_GREP_EC=$?
set -e
if [[ "$PANIC_GREP_EC" -eq 0 ]]; then
  fail_stop PRODUCT "process crash markers in evidence"
fi

{
  echo "HOST_RESULT=PASS"
  echo "STATIC_MVP_IMPORT=PASS"
  echo "REJECTION_MATRIX=PASS"
  echo "CORE_STATIC=PASS"
  echo "CORE_PROXY=PASS"
  echo "CORE_FASTCGI=PASS"
  echo "CORE_OPERATIONS=PASS"
  echo "HTTP_OR_TRANSPORT_ERRORS=0"
  echo "PROCESS_PANICS=0"
  echo "PROCESS_CRASHES=0"
  echo "ENDED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} | tee -a "$RESULT"
log "HOST_RESULT=PASS"
exit 0
