#!/usr/bin/env bash
# F-13: opt-in raw-relay E2E. The binary must be built with
# `cargo build -p exyonq --features raw-relay`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/f13-raw.XXXXXX")"
SRV_PID=""
UP_PID=""

cleanup() {
  [[ -n "$SRV_PID" ]] && kill "$SRV_PID" 2>/dev/null || true
  [[ -n "$UP_PID" ]] && kill "$UP_PID" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}
wait_port() {
  local port="$1"
  for _ in $(seq 1 200); do
    if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', $port)); s.close()" 2>/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

[[ -x "$BIN" ]] || { echo "FAIL binary not executable: $BIN"; exit 1; }

UP_PORT="$(pick_port)"
LISTEN_PORT="$(pick_port)"
CFG="$TMP/exyonq.toml"
UP_LOG="$TMP/upstream.jsonl"
SRV_LOG="$TMP/server.log"

cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["api"]

[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${UP_PORT}"
timeout_ms = 5000
EOF

python3 -u - "$UP_PORT" "$UP_LOG" <<'PY' &
import json, socket, sys
port = int(sys.argv[1]); log_path = sys.argv[2]
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port)); s.listen(32)
while True:
    conn, _ = s.accept()
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = conn.recv(65536)
        if not chunk:
            break
        data += chunk
    text = data.decode("latin-1", errors="replace")
    lines = text.split("\r\n")
    first = lines[0].split() if lines else []
    path = first[1].split("?", 1)[0] if len(first) > 1 else "/"
    headers = {}
    for line in lines[1:]:
        if not line:
            break
        if ":" in line:
            k, v = line.split(":", 1)
            headers[k.strip().lower()] = v.strip()
    with open(log_path, "a", encoding="utf-8") as f:
        f.write(json.dumps({"path": path, "headers": headers}) + "\n")
    conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nraw-ok")
    conn.close()
PY
UP_PID=$!
sleep 0.2

"$BIN" serve --config "$CFG" >"$SRV_LOG" 2>&1 &
SRV_PID=$!
if ! wait_port "$LISTEN_PORT"; then
  echo "FAIL server did not listen"
  cat "$SRV_LOG"
  exit 1
fi

python3 - "$LISTEN_PORT" <<'PY'
import socket, sys
port = int(sys.argv[1])
token = "f13"
raw = (
    f"GET /api/echo HTTP/1.1\r\n"
    f"Host: 127.0.0.1:{port}\r\n"
    f"Authorization: Bearer {token}\r\n"
    "Cookie: sid=abc\r\n"
    "Range: bytes=0-3\r\n"
    "Connection: close\r\n\r\n"
).encode()
s = socket.create_connection(("127.0.0.1", port), timeout=5)
s.sendall(raw)
data = b""
while True:
    chunk = s.recv(65536)
    if not chunk:
        break
    data += chunk
s.close()
head, _, body = data.partition(b"\r\n\r\n")
print("STATUS=" + head.split(b"\r\n", 1)[0].decode("latin-1"))
print("BODY=" + body.decode("latin-1"))
assert head.startswith(b"HTTP/1.1 200"), head
assert body == b"raw-ok", body
PY

python3 - "$UP_LOG" <<'PY'
import json, sys
rows = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
echoes = [r for r in rows if r["path"] == "/api/echo"]
assert len(echoes) == 1, rows
h = echoes[0]["headers"]
for name in ("authorization", "cookie", "range"):
    assert name not in h, (name, h)
for name in ("host", "x-forwarded-for"):
    assert name in h, (name, h)
print("RAW_RELAY_HEADER_SCOPE=AUTH_COOKIE_RANGE_ABSENT")
print("RAW_RELAY_FORWARDING=HOST_XFF_PRESENT")
PY

grep -q 'protocol=http1.1-wire' "$SRV_LOG"
grep -q 'outcome="proxy_wire"' "$SRV_LOG"
echo "RAW_RELAY_PROTOCOL=http1.1-wire"
echo "F13_RAW_RELAY_E2E=PASS"