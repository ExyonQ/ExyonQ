#!/usr/bin/env python3
"""Real HTTP upstream peer process for Cap014-L3-001.

Serves a genuine HTTP/1.1 response with declared Content-Length, writes a
prefix of the body, records sync evidence, then keeps the connection open
until the harness terminates this process (real OS process death).

Not Cap014 happy-path peer. Not Cap015. CONNECT-unavailable is out of scope.
"""
from __future__ import annotations

import argparse
import json
import os
import socket
import sys
import time
from pathlib import Path

DECLARED_CONTENT_LENGTH = 4096  # ≤ BENCH_SMALL_UPSTREAM_BODY (16 KiB)
BODY_PREFIX_LEN = 256


def read_request(conn: socket.socket) -> bytes:
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = conn.recv(4096)
        if not chunk:
            break
        buf += chunk
        if len(buf) > 65536:
            break
    return buf


def path_of(req: bytes) -> str:
    try:
        line = req.split(b"\r\n", 1)[0].decode("latin-1")
        return line.split(" ")[1]
    except (IndexError, UnicodeError):
        return "/"


def serve(listen: str, sync_path: Path, meta_path: Path) -> int:
    host, port_s = listen.rsplit(":", 1)
    port = int(port_s)
    sync_path.parent.mkdir(parents=True, exist_ok=True)

    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind((host, port))
    srv.listen(8)
    meta_path.write_text(
        json.dumps(
            {
                "UPSTREAM_IMPLEMENTATION": "scripts/reality/cap-014-l3-midbody-upstream.py",
                "UPSTREAM_PID": os.getpid(),
                "UPSTREAM_LISTEN_ADDRESS": f"{host}:{port}",
                "UPSTREAM_DECLARED_CONTENT_LENGTH": DECLARED_CONTENT_LENGTH,
                "UPSTREAM_BODY_PREFIX_LEN": BODY_PREFIX_LEN,
                "STATE": "LISTENING",
            },
            indent=2,
        )
        + "\n"
    )
    print(f"LISTENING pid={os.getpid()} addr={host}:{port}", flush=True)

    while True:
        conn, addr = srv.accept()
        try:
            conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            req = read_request(conn)
            path = path_of(req)
            if path.startswith("/ready"):
                body = b"ready\n"
                resp = (
                    b"HTTP/1.1 200 OK\r\n"
                    b"Content-Type: text/plain\r\n"
                    b"Content-Length: "
                    + str(len(body)).encode()
                    + b"\r\nConnection: close\r\n\r\n"
                    + body
                )
                conn.sendall(resp)
                conn.close()
                continue

            if not path.startswith("/midbody"):
                body = b"not-found\n"
                resp = (
                    b"HTTP/1.1 404 Not Found\r\n"
                    b"Content-Length: "
                    + str(len(body)).encode()
                    + b"\r\nConnection: close\r\n\r\n"
                    + body
                )
                conn.sendall(resp)
                conn.close()
                continue

            # Legitimate response start: declared CL, then only a prefix.
            prefix = (b"P" * BODY_PREFIX_LEN)
            assert len(prefix) == BODY_PREFIX_LEN
            assert BODY_PREFIX_LEN < DECLARED_CONTENT_LENGTH
            headers = (
                b"HTTP/1.1 200 OK\r\n"
                b"Content-Type: application/octet-stream\r\n"
                b"Content-Length: "
                + str(DECLARED_CONTENT_LENGTH).encode()
                + b"\r\n"
                b"Connection: close\r\n"
                b"\r\n"
            )
            conn.sendall(headers)
            conn.sendall(prefix)

            sync = {
                "UPSTREAM_RESPONSE_STARTED": "YES",
                "UPSTREAM_RESPONSE_STATUS": 200,
                "UPSTREAM_CONTENT_LENGTH_HEADER": DECLARED_CONTENT_LENGTH,
                "UPSTREAM_BODY_BYTES_ACTUALLY_DELIVERED": BODY_PREFIX_LEN,
                "UPSTREAM_DECLARED_CONTENT_LENGTH": DECLARED_CONTENT_LENGTH,
                "UPSTREAM_PEER_ADDR": f"{addr[0]}:{addr[1]}",
                "UPSTREAM_PID": os.getpid(),
                "PATH": path,
                "HEADERS_AND_PREFIX_FLUSHED": "YES",
                "HOLDING_CONNECTION_AWAITING_PROCESS_TERMINATION": "YES",
                "UTC": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            }
            sync_path.write_text(json.dumps(sync, indent=2) + "\n")
            print("RESPONSE_STARTED_SYNC_WRITTEN", flush=True)

            # Keep TCP connection open with incomplete body until killed.
            while True:
                time.sleep(0.2)
        except Exception as exc:  # noqa: BLE001 — peer process; log and continue
            print(f"CONN_ERROR {exc}", flush=True)
            try:
                conn.close()
            except OSError:
                pass


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--listen", required=True)
    ap.add_argument("--sync-file", required=True)
    ap.add_argument("--meta-file", required=True)
    args = ap.parse_args()
    try:
        return serve(args.listen, Path(args.sync_file), Path(args.meta_file))
    except KeyboardInterrupt:
        return 0


if __name__ == "__main__":
    sys.exit(main() or 0)
