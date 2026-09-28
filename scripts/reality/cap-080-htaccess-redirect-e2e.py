#!/usr/bin/env python3
"""htaccess overlay redirect — real binary, real TCP, file-byte oracle.

Expected status and Location come from the `.htaccess` text this script writes.
The target body is hashed from the file, not from ExyonQ.
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
BODY = b"htaccess-oracle-target-v1"


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


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


def http_exchange(port: int, path: str) -> tuple[int, str, bytes]:
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
            if not sep:
                continue
            need = 0
            for line in head.split(b"\r\n"):
                if line.lower().startswith(b"content-length:"):
                    need = int(line.split(b":", 1)[1].strip() or b"0")
            if len(body) >= need:
                break
        buf = bytes(buf)
    head, _, body = buf.partition(b"\r\n\r\n")
    text = head.decode("latin1", "replace")
    status = 0
    first = text.split("\r\n", 1)[0]
    parts = first.split()
    if len(parts) >= 2 and parts[1].isdigit():
        status = int(parts[1])
    location = ""
    for line in text.split("\r\n")[1:]:
        if line.lower().startswith("location:"):
            location = line.split(":", 1)[1].strip()
    return status, location, body


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    root = EV / "www"
    root.mkdir(parents=True, exist_ok=True)
    target = root / "new.txt"
    target.write_bytes(BODY)
    (root / ".htaccess").write_text("Redirect 301 /old /new.txt\n")
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
htaccess = "overlay"
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
        "FEATURE_ID": "htaccess-redirect",
        "ORACLE_ORIGIN": "HTACCESS_TEXT_AND_FILE_BYTES",
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else "MISSING",
        "TARGET_SHA256": sha256_bytes(BODY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
    }
    try:
        if not wait_listen(port):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": "listen timeout",
                    "LOG_TAIL": log.read_text(errors="replace")[-2000:],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3
        status, location, body = 0, "", b""
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            status, location, body = http_exchange(port, "/old")
            if status == 301:
                break
            time.sleep(0.1)
        direct_status, _, direct_body = http_exchange(port, "/new.txt")
        redirect_ok = status == 301 and location.endswith("/new.txt") and body != BODY
        target_ok = direct_status == 200 and direct_body == BODY
        result.update(
            {
                "redirect_status": status,
                "location": location,
                "redirect_body_len": len(body),
                "target_status": direct_status,
                "target_body_match": target_ok,
                "FINAL_RESULT": "PASS_REAL_E2E" if redirect_ok and target_ok else "FAIL_REAL_E2E",
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if redirect_ok and target_ok else 1
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=8)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    raise SystemExit(main())
