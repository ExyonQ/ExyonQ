#!/usr/bin/env bash
# Debug WebSocket e2e failure on Linux only.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:/root/.cargo/bin:${PATH:-}"

LISTEN="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
UP="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
TMP="$(mktemp -d)"
CFG="$TMP/exyonq.toml"
cat >"$CFG" <<EOF
config_version = 1
[[server]]
listen = "127.0.0.1:${LISTEN}"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/api/" }
upstream = "backend"
[[upstream]]
name = "backend"
target = "http://127.0.0.1:${UP}"
timeout_ms = 5000
EOF

UP_LOG="$TMP/up.log"
SRV_LOG="$TMP/srv.log"
UPSTREAM_IDENTITY=ws-up python3 - "$UP" "$UP_LOG" <<'PY' &
import json, os, socket, sys, base64, hashlib, struct
port = int(sys.argv[1])
log_path = sys.argv[2]
WS_GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

def ws_accept(key):
    return base64.b64encode(hashlib.sha1(key.encode() + WS_GUID).digest()).decode()

def recv_exact(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise EOFError("unexpected eof")
        buf += chunk
    return buf

def ws_read_frame(sock):
    hdr = recv_exact(sock, 2)
    masked = (hdr[1] & 0x80) != 0
    length = hdr[1] & 0x7F
    if length == 126:
        length = struct.unpack("!H", recv_exact(sock, 2))[0]
    elif length == 127:
        length = struct.unpack("!Q", recv_exact(sock, 8))[0]
    mask = recv_exact(sock, 4) if masked else b""
    payload = recv_exact(sock, length)
    if masked:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return payload

def ws_send_text(sock, text):
    data = text.encode()
    sock.sendall(bytes([0x81, len(data)]) + data)

s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port))
s.listen(128)
while True:
    conn, _ = s.accept()
    data = conn.recv(65536)
    text = data.decode("latin-1", errors="replace")
    lines = text.split("\r\n")
    headers = {}
    for line in lines[1:]:
        if line == "":
            break
        if ":" in line:
            k, v = line.split(":", 1)
            headers[k.strip().lower()] = v.strip()
    if headers.get("upgrade", "").lower() == "websocket":
        accept = ws_accept(headers.get("sec-websocket-key", ""))
        resp = "\r\n".join([
            "HTTP/1.1 101 Switching Protocols",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Accept: {accept}",
            "", "",
        ])
        conn.sendall(resp.encode())
        msg = ws_read_frame(conn)
        if msg is not None:
            ws_send_text(conn, msg.decode("latin-1", errors="replace"))
        conn.close()
        continue
    body = json.dumps({"ok": True}).encode()
    conn.sendall(
        b"HTTP/1.1 200 OK\r\nContent-Length: "
        + str(len(body)).encode()
        + b"\r\n\r\n"
        + body
    )
    conn.close()
PY
UP_PID=$!
sleep 0.2

cargo build -p exyonq --bin exyonq >/dev/null
RUST_LOG=debug ./target/debug/exyonq serve --config "$CFG" >"$SRV_LOG" 2>&1 &
SRV_PID=$!
for _ in $(seq 1 60); do
  if python3 -c "import socket;s=socket.create_connection(('127.0.0.1',${LISTEN}),1);s.close()" 2>/dev/null; then
    break
  fi
  sleep 0.1
done

python3 - 127.0.0.1 "$LISTEN" <<'PY'
import base64, os, socket, struct, sys

def read_ws_payload(sock, pending=b""):
    while len(pending) < 2:
        chunk = sock.recv(4096)
        if not chunk:
            raise EOFError("missing frame header")
        pending += chunk
    hdr1 = pending[1]
    pending = pending[2:]
    masked = (hdr1 & 0x80) != 0
    length = hdr1 & 0x7F
    if length == 126:
        while len(pending) < 2:
            pending += sock.recv(4096)
        length = struct.unpack("!H", pending[:2])[0]
        pending = pending[2:]
    elif length == 127:
        while len(pending) < 8:
            pending += sock.recv(4096)
        length = struct.unpack("!Q", pending[:8])[0]
        pending = pending[8:]
    mask = b""
    if masked:
        while len(pending) < 4:
            pending += sock.recv(4096)
        mask = pending[:4]
        pending = pending[4:]
    while len(pending) < length:
        pending += sock.recv(4096)
    payload = pending[:length]
    if masked:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return payload

host, port = sys.argv[1], int(sys.argv[2])
key = base64.b64encode(os.urandom(16)).decode()
req = (
    f"GET /api/ws-echo HTTP/1.1\r\n"
    f"Host: {host}:{port}\r\n"
    "Upgrade: websocket\r\n"
    "Connection: Upgrade\r\n"
    f"Sec-WebSocket-Key: {key}\r\n"
    "Sec-WebSocket-Version: 13\r\n\r\n"
)
s = socket.create_connection((host, port), timeout=5)
s.sendall(req.encode())
resp = b""
while b"\r\n\r\n" not in resp:
    chunk = s.recv(4096)
    if not chunk:
        break
    resp += chunk
print("WS_RESPONSE_BEGIN")
print(resp[:500])
print("WS_RESPONSE_END")
if b"101" not in resp.split(b"\r\n\r\n", 1)[0]:
    print("FAIL: no 101"); sys.exit(1)
pending = resp.split(b"\r\n\r\n", 1)[1] if b"\r\n\r\n" in resp else b""
msg = b"ping-kd34"
mask = os.urandom(4)
frame = bytearray([0x81, 0x80 | len(msg)])  # FIN+text, masked (clients must mask)
frame.extend(mask)
frame.extend(bytes(b ^ mask[i % 4] for i, b in enumerate(msg)))
s.sendall(frame)
s.settimeout(3)
payload = read_ws_payload(s)
print("ECHO", payload)
if payload != msg:
    print("FAIL: echo mismatch"); sys.exit(3)
print("WS_ECHO_OK")
PY

sleep 0.3
echo "--- server log tail ---"
tail -60 "$SRV_LOG" || true
kill "$SRV_PID" "$UP_PID" 2>/dev/null || true
wait "$SRV_PID" 2>/dev/null || true
wait "$UP_PID" 2>/dev/null || true
rm -rf "$TMP"
