#!/usr/bin/env python3
"""ACME HTTP-01 must not answer a token that was never registered.

Positive challenge publication needs a live ACME directory and is NOT_EXECUTED.
A 200 for an unregistered token is a failure.
"""
from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
TOKEN = "oracle-token-not-registered"


def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_listen(port: int, timeout: float = 20.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def content_length(head: bytes) -> int:
    for line in head.split(b"\r\n"):
        if line.lower().startswith(b"content-length:"):
            return int(line.split(b":", 1)[1].strip() or b"0")
    return 0


def http_get(port: int, path: str) -> tuple[int, bytes]:
    req = f"GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    with socket.create_connection(("127.0.0.1", port), timeout=5) as sock:
        sock.settimeout(5)
        sock.sendall(req.encode())
        buf = bytearray()
        while True:
            try:
                chunk = sock.recv(65536)
            except TimeoutError:
                break
            if not chunk:
                break
            buf += chunk
            head, sep, body = buf.partition(b"\r\n\r\n")
            if sep and len(body) >= content_length(head):
                break
        buf = bytes(buf)
    head, _, body = buf.partition(b"\r\n\r\n")
    status = 0
    parts = head.split(b" ", 2)
    if len(parts) >= 2 and parts[1].isdigit():
        status = int(parts[1])
    return status, body


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    root = EV / "www"
    root.mkdir(parents=True, exist_ok=True)
    (root / "index.html").write_bytes(b"acme-oracle-index")
    port = pick_port()
    cfg = EV / "cfg.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]
[[route]]
name = "site"
match = {{ path = "/" }}
root = "{root}"
"""
    )
    log = EV / "exyonq.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    result: dict = {
        "FEATURE_ID": "acme-http01-unregistered",
        "ORACLE_ORIGIN": "UNREGISTERED_TOKEN_MUST_NOT_BE_200",
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else "MISSING",
        "POSITIVE_HTTP01_PUBLICATION": "NOT_EXECUTED",
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
    }
    try:
        if not wait_listen(port):
            result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "listen timeout", "LOG_TAIL": log.read_text(errors="replace")[-1500:]})
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3
        status, body = http_get(port, f"/.well-known/acme-challenge/{TOKEN}")
        index_status, index_body = http_get(port, "/index.html")
        ok = status != 200 and index_status == 200 and index_body == b"acme-oracle-index"
        result.update(
            {
                "challenge_status": status,
                "challenge_body_len": len(body),
                "index_ok": index_status == 200 and index_body == b"acme-oracle-index",
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


if __name__ == "__main__":
    raise SystemExit(main())
