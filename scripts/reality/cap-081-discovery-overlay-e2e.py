#!/usr/bin/env python3
"""Discovery file overlay — the peer that answers is not the one named in the TOML.

Config target is peer A. EXYONQ_DISCOVERY_FILE retargets the same upstream to
peer B. The oracle is peer B's body bytes.
"""
from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
BODY_A = b"discovery-oracle-peer-a"
BODY_B = b"discovery-oracle-peer-b"


def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def serve_peer(port: int, body: bytes, stop: threading.Event) -> None:
    sock = socket.socket()
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("127.0.0.1", port))
    sock.listen(16)
    sock.settimeout(0.3)
    payload = (
        b"HTTP/1.1 200 OK\r\nContent-Length: "
        + str(len(body)).encode()
        + b"\r\nConnection: close\r\n\r\n"
        + body
    )
    while not stop.is_set():
        try:
            conn, _ = sock.accept()
        except socket.timeout:
            continue
        try:
            conn.settimeout(2)
            buf = b""
            while b"\r\n\r\n" not in buf:
                chunk = conn.recv(4096)
                if not chunk:
                    break
                buf += chunk
            if b"\r\n\r\n" in buf:
                conn.sendall(payload)
        except OSError:
            pass
        finally:
            conn.close()
    sock.close()


def wait_listen(port: int, timeout: float = 20.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def http_get(port: int, path: str) -> tuple[int, bytes]:
    req = f"GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    with socket.create_connection(("127.0.0.1", port), timeout=5) as sock:
        sock.sendall(req.encode())
        buf = b""
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            buf += chunk
    head, _, body = buf.partition(b"\r\n\r\n")
    status = 0
    parts = head.split(b" ", 2)
    if len(parts) >= 2 and parts[1].isdigit():
        status = int(parts[1])
    return status, body


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    port_a, port_b, listen = pick_port(), pick_port(), pick_port()
    stop = threading.Event()
    threads = [
        threading.Thread(target=serve_peer, args=(port_a, BODY_A, stop), daemon=True),
        threading.Thread(target=serve_peer, args=(port_b, BODY_B, stop), daemon=True),
    ]
    for thread in threads:
        thread.start()
    overlay = EV / "discovery.json"
    overlay.write_text(
        json.dumps({"upstreams": {"backend": f"http://127.0.0.1:{port_b}"}})
    )
    cfg = EV / "cfg.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["api"]

[[route]]
name = "api"
match = {{ path = "/api" }}
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:{port_a}"
timeout_ms = 2000
"""
    )
    log = EV / "exyonq.log"
    env = os.environ.copy()
    env["EXYONQ_DISCOVERY_FILE"] = str(overlay)
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
        env=env,
    )
    result: dict = {
        "FEATURE_ID": "discovery-overlay",
        "ORACLE_ORIGIN": "SECOND_PROCESS_BODY",
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else "MISSING",
        "CONFIG_TARGET": f"http://127.0.0.1:{port_a}",
        "OVERLAY_TARGET": f"http://127.0.0.1:{port_b}",
        "LIVE_REWATCH": "NOT_CLAIMED",
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
    }
    try:
        if not wait_listen(listen):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": "listen timeout",
                    "LOG_TAIL": log.read_text(errors="replace")[-2000:],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3
        status, body = http_get(listen, "/api/item")
        ok = status == 200 and body == BODY_B and body != BODY_A
        result.update(
            {
                "status": status,
                "body_is_peer_b": body == BODY_B,
                "body_is_peer_a": body == BODY_A,
                "FINAL_RESULT": "PASS_REAL_E2E" if ok else "FAIL_REAL_E2E",
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if ok else 1
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=8)
        except subprocess.TimeoutExpired:
            proc.kill()
        stop.set()


if __name__ == "__main__":
    raise SystemExit(main())
