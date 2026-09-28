#!/usr/bin/env python3
"""P1.3a soak upstream fixture: HTTP echo, streaming, slow, SSE, WebSocket echo."""
from __future__ import annotations

import base64
import hashlib
import json
import os
import socket
import struct
import threading
import time

WS_GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
IDENTITY = os.environ.get("P13A_UPSTREAM_IDENTITY", "p13a-up")
SLOW_MS = int(os.environ.get("P13A_UPSTREAM_SLOW_MS", "1500"))
STREAM_CHUNKS = int(os.environ.get("P13A_UPSTREAM_STREAM_CHUNKS", "8"))
BIND = os.environ.get("P13A_UPSTREAM_BIND", "127.0.0.1")
PORT = int(os.environ.get("P13A_UPSTREAM_PORT", "19090"))
STATE = {"requests": 0, "ws": 0, "sse": 0}


def ws_accept(key: str) -> str:
    return base64.b64encode(hashlib.sha1(key.encode() + WS_GUID).digest()).decode()


def ws_read_frame(sock: socket.socket):
    hdr = sock.recv(2)
    if len(hdr) < 2:
        return None
    masked = (hdr[1] & 0x80) != 0
    length = hdr[1] & 0x7F
    if length == 126:
        length = struct.unpack("!H", sock.recv(2))[0]
    elif length == 127:
        length = struct.unpack("!Q", sock.recv(8))[0]
    mask = sock.recv(4) if masked else b""
    payload = b""
    while len(payload) < length:
        chunk = sock.recv(length - len(payload))
        if not chunk:
            break
        payload += chunk
    if masked:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return hdr[0] & 0x0F, payload


def ws_send(sock: socket.socket, opcode: int, data: bytes):
    if len(data) < 126:
        frame = bytearray([0x80 | opcode, len(data)])
    else:
        frame = bytearray([0x80 | opcode, 126]) + struct.pack("!H", len(data))
    frame.extend(data)
    sock.sendall(frame)


def parse_request(data: bytes):
    text = data.decode("latin-1", errors="replace")
    lines = text.split("\r\n")
    parts = (lines[0] if lines else "").split()
    method = parts[0] if parts else "GET"
    target = parts[1] if len(parts) > 1 else "/"
    headers = {}
    body = b""
    for i in range(1, len(lines)):
        if lines[i] == "":
            body = data.split(b"\r\n\r\n", 1)[1] if b"\r\n\r\n" in data else b""
            break
        if ":" in lines[i]:
            k, v = lines[i].split(":", 1)
            headers[k.strip().lower()] = v.strip()
    return method, target, headers, body


def send_json(conn: socket.socket, code: int, obj: dict, head: bool = False):
    body = json.dumps(obj, separators=(",", ":")).encode()
    status = {200: "200 OK", 500: "500 Internal Server Error"}.get(code, f"{code} OK")
    hdrs = [
        f"HTTP/1.1 {status}",
        f"Content-Length: {0 if head else len(body)}",
        "Content-Type: application/json",
        "Connection: close",
        f"X-Upstream-Identity: {IDENTITY}",
    ]
    conn.sendall(("\r\n".join(hdrs) + "\r\n\r\n").encode())
    if not head and body:
        conn.sendall(body)


def handle_websocket(conn: socket.socket, headers: dict):
    STATE["ws"] += 1
    key = headers.get("sec-websocket-key", "")
    accept = ws_accept(key)
    resp = (
        "HTTP/1.1 101 Switching Protocols\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Accept: {accept}\r\n\r\n"
    )
    conn.sendall(resp.encode())
    conn.settimeout(30)
    try:
        while True:
            frame = ws_read_frame(conn)
            if frame is None:
                break
            opcode, payload = frame
            if opcode == 0x8:
                break
            if opcode in (0x1, 0x2):
                ws_send(conn, opcode, payload)
    except Exception:
        pass
    finally:
        try:
            conn.close()
        except Exception:
            pass


def handle_client(conn: socket.socket):
    try:
        data = conn.recv(65536)
        if not data:
            conn.close()
            return
        method, target, headers, body = parse_request(data)
        path = target.split("?", 1)[0]
        query = target.split("?", 1)[1] if "?" in target else ""
        STATE["requests"] += 1

        if headers.get("upgrade", "").lower() == "websocket":
            handle_websocket(conn, headers)
            return

        head = method == "HEAD"

        if path in ("/health", "/api/health"):
            send_json(conn, 200, {"ok": True, "identity": IDENTITY, "path": path}, head=head)
            return

        if path in ("/stats", "/api/stats"):
            send_json(conn, 200, dict(STATE, identity=IDENTITY), head=head)
            return

        if path.endswith("/slow") or path == "/slow":
            time.sleep(SLOW_MS / 1000.0)
            send_json(conn, 200, {"ok": True, "slow_ms": SLOW_MS, "identity": IDENTITY}, head=head)
            return

        if path in ("/api/stream", "/stream") or path.endswith("/stream"):
            STATE["sse"] += 1
            qs = {}
            for part in query.split("&"):
                if "=" in part:
                    k, v = part.split("=", 1)
                    qs[k] = v
            events = int(qs.get("n", "5"))
            delay = float(qs.get("delay", "0.05"))
            hdr = (
                "HTTP/1.1 200 OK\r\n"
                "Content-Type: text/event-stream\r\n"
                "Cache-Control: no-cache\r\n"
                "Transfer-Encoding: chunked\r\n"
                f"X-Upstream-Identity: {IDENTITY}\r\n\r\n"
            )
            conn.sendall(hdr.encode())
            for i in range(events):
                data_b = f"data: event-{i}\n\n".encode()
                conn.sendall(f"{len(data_b):x}\r\n".encode() + data_b + b"\r\n")
                time.sleep(delay)
            conn.sendall(b"0\r\n\r\n")
            conn.close()
            return

        if path.endswith("/stream-bytes") or path == "/stream-bytes":
            hdr = (
                "HTTP/1.1 200 OK\r\n"
                "Content-Type: application/octet-stream\r\n"
                "Transfer-Encoding: chunked\r\n"
                f"X-Upstream-Identity: {IDENTITY}\r\n\r\n"
            )
            conn.sendall(hdr.encode())
            for i in range(STREAM_CHUNKS):
                data_b = (f"CHUNK-{i}-" + ("x" * 256)).encode()
                conn.sendall(f"{len(data_b):x}\r\n".encode() + data_b + b"\r\n")
                time.sleep(0.02)
            conn.sendall(b"0\r\n\r\n")
            conn.close()
            return

        if path.endswith("/big") or path == "/big":
            body_out = (b"P13A_BIG_" + (b"z" * 1024)) * 256
            hdrs = [
                "HTTP/1.1 200 OK",
                f"Content-Length: {0 if head else len(body_out)}",
                "Content-Type: application/octet-stream",
                "Connection: close",
                f"X-Upstream-Identity: {IDENTITY}",
            ]
            conn.sendall(("\r\n".join(hdrs) + "\r\n\r\n").encode())
            if not head:
                conn.sendall(body_out)
            conn.close()
            return

        payload = {
            "identity": IDENTITY,
            "method": method,
            "path": path,
            "query": query,
            "body_len": len(body),
            "body_marker": "P13A_BODY_OK" if b"P13A_BODY_OK" in body or body else "",
            "body": body.decode("latin-1", errors="replace")[:200],
            "xff": headers.get("x-forwarded-for", ""),
            "host": headers.get("host", ""),
        }
        if "P13A_BODY_OK" in payload["body"]:
            payload["body_marker"] = "P13A_BODY_OK"
        send_json(conn, 200, payload, head=head)
    except Exception:
        try:
            conn.close()
        except Exception:
            pass


def main():
    s = socket.socket()
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind((BIND, PORT))
    s.listen(256)
    print(f"P13A_UPSTREAM listen={BIND}:{PORT} identity={IDENTITY}", flush=True)
    while True:
        conn, _ = s.accept()
        threading.Thread(target=handle_client, args=(conn,), daemon=True).start()


if __name__ == "__main__":
    main()
