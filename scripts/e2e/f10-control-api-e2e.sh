#!/usr/bin/env bash
# Real F-10 control-plane E2E. Runs the built exyonq binary with an isolated
# config/control/API socket, then exercises the HTTP control API over Unix.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED platform=$(uname -s)"
  exit 2
fi

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
[[ -x "$EXYONQ_BIN" ]] || { echo "RESULT=BLOCKED missing=$EXYONQ_BIN"; exit 3; }

TMP="$(mktemp -d)"
PID=""
cleanup() {
  if [[ -n "$PID" ]] && kill -0 "$PID" 2>/dev/null; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
DOCROOT="$TMP/www"
API_DIR="$TMP/api"
API_SOCKET="$API_DIR/control.sock"
mkdir -p "$DOCROOT"
printf 'f10\n' >"$DOCROOT/index.html"
cat >"$TMP/exyonq.toml" <<EOF
config_version = 1
[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "$DOCROOT"
EOF

FULL="exq_sk_v1_full_$(date +%s)_a"
READ="exq_sk_v1_read_$(date +%s)_b"
WRITE="exq_sk_v1_write_$(date +%s)_c"
ROTATED="exq_sk_v1_rotated_$(date +%s)_d"
printf '%s\n' "$FULL" >"$TMP/token"
chmod 0600 "$TMP/token"

EXYONQ_CONFIG="$TMP/exyonq.toml" \
EXYONQ_CONTROL_SOCKET="$TMP/control.sock" \
EXYONQ_API_SOCKET="$API_SOCKET" \
EXYONQ_SERVER_TOKEN_FILE="$TMP/token" \
EXYONQ_SERVER_READ_TOKEN="$READ" \
EXYONQ_SERVER_WRITE_TOKEN="$WRITE" \
RUST_LOG=info "$EXYONQ_BIN" serve -c "$TMP/exyonq.toml" >"$TMP/ex.log" 2>&1 &
PID=$!

for _ in $(seq 1 100); do
  [[ -S "$API_SOCKET" ]] && break
  if ! kill -0 "$PID" 2>/dev/null; then
    echo "FAIL server exited"
    cat "$TMP/ex.log"
    exit 1
  fi
  sleep 0.1
done
[[ -S "$API_SOCKET" ]] || { echo "FAIL api socket missing"; cat "$TMP/ex.log"; exit 1; }

code() { # method path token header?
  local method="$1" path="$2" token="${3:-}" out="$TMP/out"
  local -a args=(-sS -o "$out" -w '%{http_code}' -X "$method" --unix-socket "$API_SOCKET" "http://localhost${path}")
  [[ -n "$token" ]] && args+=(-H "X-ExyonQ-Server-Token: $token")
  curl "${args[@]}"
}

DIR_MODE="$(stat -c '%a' "$API_DIR")"
SOCK_MODE="$(stat -c '%a' "$API_SOCKET")"
NO_TOKEN="$(code GET /api/v1/stats)"
READ_GET="$(code GET /api/v1/stats "$READ")"
READ_POST="$(code POST /api/v1/config/vhosts "$READ")"
WRITE_GET="$(code GET /api/v1/stats "$WRITE")"
WRITE_POST="$(code POST /api/v1/config/vhosts "$WRITE")"

printf '%s\n' "$ROTATED" >"$TMP/token.new"
chmod 0600 "$TMP/token.new"
mv "$TMP/token.new" "$TMP/token"
OLD_AFTER="$(code GET /api/v1/stats "$FULL")"
NEW_AFTER="$(code GET /api/v1/stats "$ROTATED")"
chmod 0644 "$TMP/token"
INSECURE_MODE="$(code GET /api/v1/stats "$ROTATED")"
chmod 0600 "$TMP/token"

PEER_CODE="dropped"
PRIV=""
if [[ "$(id -u)" != "0" ]] && sudo -n true 2>/dev/null; then
  PRIV="sudo -n"
fi
if [[ "$(id -u)" == "0" || -n "$PRIV" ]] && command -v setpriv >/dev/null 2>&1; then
  # The live socket is owner-only; make it reachable so the SO_PEERCRED gate
  # (not the filesystem mode) is the thing that rejects the foreign uid.
  $PRIV chmod 0755 "$API_DIR"
  $PRIV chmod 0666 "$API_SOCKET"
  PEER_CODE="$($PRIV setpriv --reuid=nobody --regid=nogroup --clear-groups \
    curl -sS -o /dev/null -w '%{http_code}' --unix-socket "$API_SOCKET" \
    http://localhost/api/v1/stats -H "X-ExyonQ-Server-Token: $ROTATED" 2>/dev/null || echo blocked)"
fi

OK=1
[[ "$DIR_MODE" == "700" ]] || { echo "FAIL dir_mode=$DIR_MODE"; OK=0; }
[[ "$SOCK_MODE" == "600" ]] || { echo "FAIL sock_mode=$SOCK_MODE"; OK=0; }
[[ "$NO_TOKEN" == "401" ]] || { echo "FAIL no_token=$NO_TOKEN"; OK=0; }
[[ "$READ_GET" == "200" ]] || { echo "FAIL read_get=$READ_GET"; OK=0; }
[[ "$READ_POST" == "403" ]] || { echo "FAIL read_post=$READ_POST"; OK=0; }
[[ "$WRITE_GET" == "403" ]] || { echo "FAIL write_get=$WRITE_GET"; OK=0; }
[[ "$WRITE_POST" == "400" ]] || { echo "FAIL write_post=$WRITE_POST"; OK=0; }
[[ "$OLD_AFTER" == "401" ]] || { echo "FAIL old_after=$OLD_AFTER"; OK=0; }
[[ "$NEW_AFTER" == "200" ]] || { echo "FAIL new_after=$NEW_AFTER"; OK=0; }
[[ "$INSECURE_MODE" == "401" ]] || { echo "FAIL insecure_mode=$INSECURE_MODE"; OK=0; }
if [[ "$PEER_CODE" == *000* || "$PEER_CODE" == *blocked* ]]; then
  PEER_CODE="rejected"
else
  echo "FAIL peercred_nobody=$PEER_CODE"
  OK=0
fi

echo "F10_DIR_MODE=$DIR_MODE"
echo "F10_SOCK_MODE=$SOCK_MODE"
echo "F10_NO_TOKEN=$NO_TOKEN"
echo "F10_READ_GET=$READ_GET"
echo "F10_READ_POST=$READ_POST"
echo "F10_WRITE_GET=$WRITE_GET"
echo "F10_WRITE_POST=$WRITE_POST"
echo "F10_OLD_AFTER_ROTATE=$OLD_AFTER"
echo "F10_NEW_AFTER_ROTATE=$NEW_AFTER"
echo "F10_INSECURE_TOKEN_MODE=$INSECURE_MODE"
echo "F10_PEERCRED_NOBODY=$PEER_CODE"
if [[ "$OK" == "1" ]]; then
  echo "F10=PASS"
  exit 0
fi
echo "F10=FAIL"
exit 1
