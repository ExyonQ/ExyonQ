#!/usr/bin/env bash
# F-13: opt-in CFD dataplane E2E. Build the binaries with
# `cargo build -p exyonq --features cfd` and
# `cargo build -p exyonq-cfd-dataplane --bin exyonq-dataplane`.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
DP_BIN="${EXYONQ_DATAPLANE_BIN:-$ROOT/target/debug/exyonq-dataplane}"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/f13-cfd.XXXXXX")"
SRV_PID=""
UP_PID=""

cleanup() {
  if [[ -n "$SRV_PID" ]]; then
    pkill -P "$SRV_PID" 2>/dev/null || true
    kill "$SRV_PID" 2>/dev/null || true
  fi
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
  for _ in $(seq 1 300); do
    if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', $port)); s.close()" 2>/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

[[ -x "$BIN" ]] || { echo "FAIL binary not executable: $BIN"; exit 1; }
[[ -x "$DP_BIN" ]] || { echo "FAIL dataplane not executable: $DP_BIN"; exit 1; }

UP_PORT="$(pick_port)"
CFD_PORT="$(pick_port)"
LISTEN_PORT="$(pick_port)"
CFG="$TMP/exyonq.toml"
ROUTES="$TMP/routes.txt"
GEN="$TMP/gen"
UP_LOG="$TMP/upstream.jsonl"
SRV_LOG="$TMP/server.log"

cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${LISTEN_PORT}"
routes = ["product"]

[[route]]
name = "product"
match = { path = "/product", exact = true }
respond = { status = 200, body = "product-ok" }
EOF

printf 'cfd.test|/api/echo|127.0.0.1:%s|127.0.0.1\n' "$UP_PORT" >"$ROUTES"

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
    conn.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\ncfd-ok")
    conn.close()
PY
UP_PID=$!
sleep 0.2

EXYONQ_COMPETITIVE_H1_DATAPLANE=1 \
EXYONQ_CFD_LISTEN="127.0.0.1:${CFD_PORT}" \
EXYONQ_CFD_GEN_DIR="$GEN" \
EXYONQ_CFD_ROUTES="$ROUTES" \
EXYONQ_CFD_SHARDS=1 \
EXYONQ_DATAPLANE_BIN="$DP_BIN" \
"$BIN" serve --config "$CFG" >"$SRV_LOG" 2>&1 &
SRV_PID=$!

if ! wait_port "$LISTEN_PORT"; then
  echo "FAIL product did not listen"
  cat "$SRV_LOG"
  exit 1
fi
if ! wait_port "$CFD_PORT"; then
  echo "FAIL CFD dataplane did not listen"
  cat "$SRV_LOG"
  exit 1
fi

python3 - "$CFD_PORT" <<'PY'
import socket, sys
port = int(sys.argv[1])
token = "cfd"
raw = (
    "GET /api/echo HTTP/1.1\r\n"
    "Host: cfd.test\r\n"
    "X-CFD-Check: yes\r\n"
    f"Authorization: Bearer {token}\r\n"
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
print("CFD_STATUS=" + head.split(b"\r\n", 1)[0].decode("latin-1"))
print("CFD_BODY=" + body.decode("latin-1"))
assert head.startswith(b"HTTP/1.1 200"), head
assert body == b"cfd-ok", body
PY

python3 - "$LISTEN_PORT" <<'PY'
import socket, sys
port = int(sys.argv[1])
raw = (
    f"GET /product HTTP/1.1\r\n"
    f"Host: 127.0.0.1:{port}\r\n"
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
print("PRODUCT_STATUS=" + head.split(b"\r\n", 1)[0].decode("latin-1"))
print("PRODUCT_BODY=" + body.decode("latin-1"))
assert head.startswith(b"HTTP/1.1 200"), head
assert body == b"product-ok", body
PY

python3 - "$UP_LOG" <<'PY'
import json, sys
rows = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
echoes = [r for r in rows if r["path"] == "/api/echo"]
assert len(echoes) == 1, rows
h = echoes[0]["headers"]
assert h.get("x-cfd-check") == "yes", h
assert h.get("authorization") == f"Bearer {token}", h
print("CFD_ROUTE_UPSTREAM=MATCHED")
print("CFD_HEADERS=FORWARDED")
PY

grep -q 'competitive H1 dataplane READY' "$SRV_LOG"
grep -q 'READY shards=1' "$SRV_LOG"
echo "F13_CFD_E2E=PASS"