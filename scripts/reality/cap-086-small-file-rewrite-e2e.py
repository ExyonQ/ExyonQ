#!/usr/bin/env python3
"""Small-file responses must observe a later rewrite and a later rename.

The file is  under the 4096-byte cache. The oracle sleeps 50 ms, past the
1 ms identity window, then compares bytes the script wrote. A slow read of a
larger file checks that the body still matches; it does not isolate EAGAIN.
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
FIRST = b"small-oracle-first"
SECOND = b"small-oracle-second-rewritten"
THIRD = b"small-oracle-third-renamed"
LARGE = bytes([0x5A]) * (256 * 1024)


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
    with socket.create_connection(("127.0.0.1", port), timeout=10) as sock:
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


def slow_get(port: int, path: str) -> bytes:
    req = f"GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    with socket.create_connection(("127.0.0.1", port), timeout=15) as sock:
        sock.sendall(req.encode())
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024)
        buf = b""
        while True:
            time.sleep(0.01)
            chunk = sock.recv(512)
            if not chunk:
                break
            buf += chunk
    _, _, body = buf.partition(b"\r\n\r\n")
    return body


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    root = EV / "www"
    root.mkdir(parents=True, exist_ok=True)
    small = root / "small.txt"
    small.write_bytes(FIRST)
    (root / "large.bin").write_bytes(LARGE)
    port = pick_port()
    cfg = EV / "cfg.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]
[[route]]
name = "site"
match = {{ path = "/site" }}
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
        "FEATURE_ID": "small-file-rewrite-rename",
        "ORACLE_ORIGIN": "FILE_BYTES_AFTER_SLEEP",
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else "MISSING",
        "EAGAIN_BRANCH": "NOT_ISOLATED",
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
    }
    try:
        if not wait_listen(port):
            result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "listen timeout", "LOG_TAIL": log.read_text(errors="replace")[-1500:]})
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3
        st1, b1 = http_get(port, "/site/small.txt")
        small.write_bytes(SECOND)
        time.sleep(0.05)
        st2, b2 = http_get(port, "/site/small.txt")
        replacement = root / "small.txt.next"
        replacement.write_bytes(THIRD)
        os.rename(replacement, small)
        time.sleep(0.05)
        st3, b3 = http_get(port, "/site/small.txt")
        large_body = slow_get(port, "/site/large.bin")
        ok = (
            st1 == 200 and b1 == FIRST
            and st2 == 200 and b2 == SECOND
            and st3 == 200 and b3 == THIRD
            and large_body == LARGE
        )
        result.update(
            {
                "first_match": b1 == FIRST,
                "rewrite_match": b2 == SECOND,
                "rename_match": b3 == THIRD,
                "slow_large_match": large_body == LARGE,
                "statuses": [st1, st2, st3],
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
